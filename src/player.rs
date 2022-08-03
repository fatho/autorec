//! # Playing MIDI files
//!
//! This is currently a hacky implementation that relies on invoking `aplaymidi` for convenience.
//! Eventually, it would be nice to have a working implementation to talk directly to the platform's
//! MIDI API. Unfortunately, this isn't entirely trivial within `tokio`.

use std::{pin::Pin, process::Stdio, sync::Arc};

use lazy_static::lazy_static;
use tokio::{
    select,
    sync::{broadcast, oneshot, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error};

#[derive(Debug)]
pub struct MidiPlayer {
    cancellation_token: CancellationToken,
}

lazy_static!(
    /// MIDI file that sends a single GM Reset message.
    static ref GM_RESET_MESSAGE_MID: Vec<u8> = {
        let mut smf = midly::Smf::new(midly::Header::new(
            midly::Format::SingleTrack,
            // Timing doesn't matter since we just send a single message
            midly::Timing::Metrical(midly::num::u15::new(96)),
        ));
        let track = vec![
            midly::TrackEvent {
                delta: 0.into(),
                // `GM Reset` message
                kind: midly::TrackEventKind::SysEx(&[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]),
            },
            midly::TrackEvent {
                delta: 0.into(),
                kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
            }
        ];
        smf.tracks.push(track);

        let mut output = Vec::new();
        smf.write_std(&mut output).expect("Writing to a vector shouldn't fail");
        output
    };
);

impl MidiPlayer {
    pub async fn new(
        output: String,
        source: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    ) -> std::io::Result<(Self, oneshot::Receiver<()>)> {
        // Spawn a new process for the given play request
        let mut proc = spawn_aplaymidi(output.as_str(), 2).await?;

        let stdin = proc.stdin.take().unwrap();
        tokio::spawn(feed_aplaymidi(stdin, source));

        let cancellation_token = CancellationToken::new();
        let (completed_tx, completed_rx) = oneshot::channel::<()>();

        tokio::spawn({
            let cancellation_token = cancellation_token.clone();
            let output = output.clone();
            async move {
                select! {
                    _ = cancellation_token.cancelled() => {
                        let _ = proc.kill().await;
                        reset_output(&output).await;
                    }
                    _ = proc.wait() => {
                        // Normal exit
                        let _ = completed_tx.send(());
                    }
                }
            }
        });

        Ok((Self { cancellation_token }, completed_rx))
    }

    pub fn stop(&self) {
        self.cancellation_token.cancel();
    }
}

impl Drop for MidiPlayer {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
    }
}

async fn spawn_aplaymidi(output: &str, delay: u32) -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new("aplaymidi")
        .arg("-p")
        .arg(output)
        .arg("-d")
        .arg(delay.to_string())
        .arg("-") // read from stdin
        .stdin(Stdio::piped())
        .spawn()
}

async fn feed_aplaymidi(
    mut stdin: tokio::process::ChildStdin,
    mut source: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
) {
    match tokio::io::copy(&mut source, &mut stdin).await {
        Ok(count) => debug!("Played {count} MIDI bytes"),
        Err(err) => {
            error!("Failed to send data to aplaymidi: {err}")
        }
    }
}

async fn reset_output(output: &str) {
    if let Ok(mut reset_cmd) = spawn_aplaymidi(output, 0).await {
        let source = Box::pin(GM_RESET_MESSAGE_MID.as_slice());
        let stdin = reset_cmd.stdin.take().unwrap();
        feed_aplaymidi(stdin, source).await;
        let _ = reset_cmd.wait().await;
    }
}

#[derive(Debug)]
pub struct MidiPlayQueue<T> {
    shared: Arc<Mutex<QueueShared<T>>>,
    player: Option<(MidiPlayer, JoinHandle<()>)>,
    tx: Arc<broadcast::Sender<QueueEvent<T>>>,
}

#[derive(Debug)]
struct QueueShared<T> {
    current: Option<T>,
}

#[derive(Debug, Clone)]
pub enum QueueEvent<T> {
    PlaybackStart(T),
    PlaybackStop(T),
}

impl<T: Clone + Send + 'static> MidiPlayQueue<T> {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(16);

        Self {
            shared: Arc::new(Mutex::new(QueueShared { current: None })),
            player: None,
            tx: Arc::new(tx),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<QueueEvent<T>> {
        self.tx.subscribe()
    }

    pub async fn play(
        &mut self,
        token: T,
        output: String,
        source: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
    ) -> std::io::Result<()> {
        if let Some((player, waiter)) = self.player.take() {
            player.stop();
            let _ = waiter.await;
        }

        let (player, completed) = MidiPlayer::new(output, source).await?;

        let _ = self.tx.send(QueueEvent::PlaybackStart(token.clone()));

        {
            let mut state = self.shared.lock().await;
            state.current = Some(token.clone());
        }

        let waiter = tokio::spawn({
            let tx = self.tx.clone();
            let shared = self.shared.clone();
            async move {
                // Wait for player process to stop
                let _ = completed.await;
                {
                    let mut state = shared.lock().await;
                    state.current = None;
                }
                // Ignore errors, we don't care if anyone listens
                let _ = tx.send(QueueEvent::PlaybackStop(token));
            }
        });

        self.player = Some((player, waiter));

        Ok(())
    }

    pub async fn stop(&mut self) {
        if let Some((player, waiter)) = self.player.take() {
            player.stop();
            let _ = waiter.await;
        }
    }

    pub async fn current(&self) -> Option<T> {
        let state = self.shared.lock().await;
        state.current.clone()
    }
}
