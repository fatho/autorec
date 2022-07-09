//! # Playing MIDI files
//!
//! This is currently a hacky implementation that relies on invoking `aplaymidi` for convenience.
//! Eventually, it would be nice to have a working implementation to talk directly to the platform's
//! MIDI API. Unfortunately, this isn't entirely trivial within `tokio`.

use std::{
    pin::Pin,
    process::Stdio,
    sync::{atomic::AtomicBool, Arc},
};

use lazy_static::lazy_static;
use tokio::{select, sync::mpsc::Sender};
use tracing::{debug, error};

pub struct MidiPlayer {
    is_playing: Arc<AtomicBool>,
    sender: Sender<Request>,
}

lazy_static!(
    /// MIDI file that sends a single GM Reset message.
    static ref GM_RESET_MESSAGE_MID: Vec<u8> = {
        let mut smf = midly::Smf::new(midly::Header::new(
            midly::Format::SingleTrack,
            midly::Timing::Metrical(midly::num::u15::new(96)),
        ));
        let mut track = Vec::new();
        track.push(midly::TrackEvent {
            delta: 0.into(),
            // `GM Reset` message
            kind: midly::TrackEventKind::SysEx(&[0xF0, 0x7E, 0x7F, 0x09, 0x01, 0xF7]),
        });
        track.push(midly::TrackEvent {
            delta: 0.into(),
            kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
        });
        smf.tracks.push(track);

        let mut output = Vec::new();
        smf.write_std(&mut output).expect("Writing to a vector shouldn't fail");
        output
    };
);

enum Request {
    Play {
        source: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
        output: String,
    },
    Stop,
}

impl MidiPlayer {
    pub fn new() -> Self {
        let (sender, mut receiver) = tokio::sync::mpsc::channel::<Request>(4);
        let is_playing = Arc::new(AtomicBool::new(false));

        let is_playing_supervisor = is_playing.clone();
        tokio::spawn(async move {
            let mut running_process: Option<(String, tokio::process::Child)> = None;
            loop {
                let request = if let Some((_, process)) = running_process.as_mut() {
                    select! {
                        _ = process.wait() => { continue; }
                        req = receiver.recv() => {
                            let _ = process.kill().await; // TODO: log kill failure
                            let (output, _) = running_process.take().unwrap();

                            // Reset output
                            // TODO: log reset failures
                            if let Ok(mut reset_cmd) = spawn_aplaymidi(output.as_str()).await {
                                let source = Box::pin(GM_RESET_MESSAGE_MID.as_slice());
                                let stdin = reset_cmd.stdin.take().unwrap();
                                feed_aplaymidi(stdin, source).await;
                                let _ = reset_cmd.wait().await;
                            }

                            is_playing_supervisor.store(false, std::sync::atomic::Ordering::SeqCst);

                            req
                        }
                    }
                } else {
                    receiver.recv().await
                };

                match request {
                    Some(request) => match request {
                        Request::Play { source, output } => {
                            // Spawn a new process for the given play request
                            let proc = spawn_aplaymidi(output.as_str()).await;

                            match proc {
                                Ok(mut proc) => {
                                    is_playing_supervisor
                                        .store(true, std::sync::atomic::Ordering::SeqCst);

                                    let stdin = proc.stdin.take().unwrap();
                                    tokio::spawn(feed_aplaymidi(stdin, source));

                                    running_process = Some((output, proc));
                                }
                                Err(err) => {
                                    error!("Failed to spawn aplaymidi: {err}")
                                }
                            }
                        }
                        Request::Stop => {
                            // Either nothing is running or we killed the process above already
                            continue;
                        }
                    },
                    None => break,
                }
            }
        });

        Self { sender, is_playing }
    }

    pub async fn play<R: tokio::io::AsyncRead + Send + 'static>(
        &self,
        output: String,
        midi_file_reader: R,
    ) {
        let pinned_reader = Box::pin(midi_file_reader);
        if self
            .sender
            .send(Request::Play {
                source: pinned_reader,
                output,
            })
            .await
            .is_err()
        {
            panic!("BUG: receiver shouldn't hang up before sender");
        }
    }

    pub async fn stop(&self) {
        if self.sender.send(Request::Stop).await.is_err() {
            panic!("BUG: receiver shouldn't hang up before sender");
        }
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(std::sync::atomic::Ordering::SeqCst)
    }
}

async fn spawn_aplaymidi(output: &str) -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new("aplaymidi")
        .arg("-p")
        .arg(output)
        .arg("-") // read from stdin
        .stdin(Stdio::piped())
        .spawn()
}

async fn feed_aplaymidi(mut stdin: tokio::process::ChildStdin, mut source: Pin<Box<dyn tokio::io::AsyncRead + Send>>) -> () {
    match tokio::io::copy(&mut source, &mut stdin).await {
        Ok(count) => debug!("Played {count} MIDI bytes"),
        Err(err) => {
            error!("Failed to send data to aplaymidi: {err}")
        }
    }
}
