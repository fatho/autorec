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
use tokio::{select, sync::{mpsc::Sender, oneshot}, process::Child};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

pub struct MidiPlayer {
    output: String,
    cancellation_token: CancellationToken,
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

impl MidiPlayer {
    pub async fn new(output: String, source: Pin<Box<dyn tokio::io::AsyncRead + Send>>) -> std::io::Result<(Self, oneshot::Receiver<()>)> {
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
        }});

        Ok((Self {
            output,
            cancellation_token,
        }, completed_rx))
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

// enum Request {
//     Play {
//         source: Pin<Box<dyn tokio::io::AsyncRead + Send>>,
//         output: String,
//     },
//     Stop,
// }

// impl MidiPlayer {
//     pub fn new() -> Self {
//         let (sender, mut receiver) = tokio::sync::mpsc::channel::<Request>(4);
//         let is_playing = Arc::new(AtomicBool::new(false));

//         let is_playing_supervisor = is_playing.clone();
//         tokio::spawn(async move {
//             let mut running_process: Option<(String, tokio::process::Child)> = None;
//             loop {
//                 let request = if let Some((_, process)) = running_process.as_mut() {
//                     select! {
//                         _ = process.wait() => {
//                             info!("Playback finished");
//                             running_process = None;
//                             is_playing_supervisor.store(false, std::sync::atomic::Ordering::SeqCst);
//                             continue;
//                         }
//                         req = receiver.recv() => {
//                             let _ = process.kill().await; // TODO: log kill failure
//                             let (output, _) = running_process.take().unwrap();

//                             info!("Playback cancelled");

//                             // Reset output
//                             // TODO: log reset failures
//                             if let Ok(mut reset_cmd) = spawn_aplaymidi(output.as_str(), 0).await {
//                                 let source = Box::pin(GM_RESET_MESSAGE_MID.as_slice());
//                                 let stdin = reset_cmd.stdin.take().unwrap();
//                                 feed_aplaymidi(stdin, source).await;
//                                 let _ = reset_cmd.wait().await;
//                             }

//                             is_playing_supervisor.store(false, std::sync::atomic::Ordering::SeqCst);

//                             req
//                         }
//                     }
//                 } else {
//                     receiver.recv().await
//                 };

//                 match request {
//                     Some(request) => match request {
//                         Request::Play { source, output } => {
//                             // Spawn a new process for the given play request
//                             let proc = spawn_aplaymidi(output.as_str(), 2).await;

//                             match proc {
//                                 Ok(mut proc) => {
//                                     is_playing_supervisor
//                                         .store(true, std::sync::atomic::Ordering::SeqCst);

//                                     let stdin = proc.stdin.take().unwrap();
//                                     tokio::spawn(feed_aplaymidi(stdin, source));

//                                     running_process = Some((output, proc));
//                                 }
//                                 Err(err) => {
//                                     error!("Failed to spawn aplaymidi: {err}")
//                                 }
//                             }
//                         }
//                         Request::Stop => {
//                             // Either nothing is running or we killed the process above already
//                             continue;
//                         }
//                     },
//                     None => break,
//                 }
//             }
//         });

//         Self { sender, is_playing }
//     }

//     pub async fn play<R: tokio::io::AsyncRead + Send + 'static>(
//         &self,
//         output: String,
//         midi_file_reader: R,
//     ) {
//         let pinned_reader = Box::pin(midi_file_reader);
//         if self
//             .sender
//             .send(Request::Play {
//                 source: pinned_reader,
//                 output,
//             })
//             .await
//             .is_err()
//         {
//             panic!("BUG: receiver shouldn't hang up before sender");
//         }
//     }

//     pub async fn stop(&self) {
//         if self.sender.send(Request::Stop).await.is_err() {
//             panic!("BUG: receiver shouldn't hang up before sender");
//         }
//     }

//     pub fn is_playing(&self) -> bool {
//         self.is_playing.load(std::sync::atomic::Ordering::SeqCst)
//     }
// }

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

async fn feed_aplaymidi(mut stdin: tokio::process::ChildStdin, mut source: Pin<Box<dyn tokio::io::AsyncRead + Send>>) -> () {
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
