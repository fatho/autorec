use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use tracing::info;

use crate::{
    args::Args,
    midi::{Device, DeviceInfo, MidiEvent, RecordEvent},
};

pub type AppStateRef = Arc<Mutex<AppState>>;

#[derive(Default, Debug)]
pub struct AppState {
    pub devices: HashMap<Device, DeviceInfo>,
    pub connected_device: Option<Device>,
    pub song_dir: PathBuf,
    pub player: Option<std::process::Child>,
}

impl AppState {
    pub fn new(cfg: &Args) -> Self {
        Self {
            devices: Default::default(),
            connected_device: None,
            song_dir: cfg.song_directory.clone(),
            player: None,
        }
    }

    pub fn new_shared(cfg: &Args) -> AppStateRef {
        Arc::new(Mutex::new(AppState::new(cfg)))
    }

    pub fn query_songs(&self) -> std::io::Result<Vec<String>> {
        let mut songs = Vec::new();

        for song in std::fs::read_dir(&self.song_dir)? {
            if let Some(name) = song?.file_name().to_str().and_then(|name| name.strip_suffix(".mid")) {
                songs.push(name.to_owned())
            }
        }

        Ok(songs)
    }

    pub fn store_song(&mut self, name: &str, events: Vec<RecordEvent>) -> std::io::Result<()> {
        let mut filepath = self.song_dir.join(name);
        filepath.set_extension("mid");

        let mut smf = midly::Smf::new(midly::Header::new(
            midly::Format::SingleTrack,
            midly::Timing::Metrical(midly::num::u15::new(96)),
        ));
        let mut track = Vec::new();
        let mut last_time = events.first().map_or(0, |rev| rev.timestamp);

        for event in events.iter() {
            let delta = event.timestamp - last_time;
            last_time = event.timestamp;

            track.push(midly::TrackEvent {
                delta: midly::num::u28::new(delta),
                kind: match event.payload {
                    MidiEvent::NoteOn {
                        channel,
                        note,
                        velocity,
                    } => midly::TrackEventKind::Midi {
                        channel: channel.into(),
                        message: midly::MidiMessage::NoteOn {
                            key: note.into(),
                            vel: velocity.into(),
                        },
                    },
                    MidiEvent::NoteOff { channel, note } => midly::TrackEventKind::Midi {
                        channel: channel.into(),
                        message: midly::MidiMessage::NoteOff {
                            key: note.into(),
                            vel: 0.into(),
                        },
                    },
                    MidiEvent::ControlChange {
                        channel,
                        controller,
                        value,
                    } => midly::TrackEventKind::Midi {
                        channel: channel.into(),
                        message: midly::MidiMessage::Controller {
                            controller: (controller as u8).into(),
                            value: (value as u8).into(),
                        },
                    },
                },
            })
        }
        track.push(midly::TrackEvent {
            delta: 0.into(),
            kind: midly::TrackEventKind::Meta(midly::MetaMessage::EndOfTrack),
        });
        smf.tracks.push(track);

        smf.save(filepath)
    }

    pub fn play_song(&mut self, name: String) -> std::io::Result<()> {
        let mut filepath = self.song_dir.join(name);
        filepath.set_extension("mid");

        if let Some(dev) = self.connected_device.as_ref() {
            info!("Playing {}", filepath.display());

            if let Some(mut previous_player) = self.player.take() {
                // TODO: should probably have a separate thread for `wait`ing.
                if previous_player.try_wait()?.is_none() {
                    use nix::unistd::Pid;
                    use nix::sys::signal;
                    // still running, stop it
                    let _ = signal::kill(Pid::from_raw(previous_player.id() as i32), signal::SIGINT);
                    previous_player.wait()?;
                }
            }

            // hackedy hack
            let player = std::process::Command::new("aplaymidi")
                .arg("-p")
                .arg(dev.id())
                .arg(filepath)
                .spawn()?;

            self.player = Some(player);

            Ok(())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "No device for playing song"))
        }
    }

    pub fn stop_song(&mut self) -> std::io::Result<()> {
        if let Some(mut previous_player) = self.player.take() {
            if previous_player.try_wait()?.is_none() {
                use nix::unistd::Pid;
                use nix::sys::signal;
                // still running, stop it
                let _ = signal::kill(Pid::from_raw(previous_player.id() as i32), signal::SIGINT);
                previous_player.wait()?;
            }
            Ok(())
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Not currently playing"))
        }
    }
}

pub struct RecorderState {
    pub device: Device,
}
