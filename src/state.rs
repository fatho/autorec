use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use tracing::info;

use crate::{
    args::Args,
    midi::{Device, DeviceInfo, MidiEvent, RecordEvent},
    player::MidiPlayer,
};

pub type AppRef = Arc<App>;

pub struct App {
    state: RwLock<AppState>,
    player: MidiPlayer,
}

pub struct AppState {
    pub devices: HashMap<Device, DeviceInfo>,
    pub connected_device: Option<Device>,
    pub song_dir: PathBuf,
    pub last_song: Option<String>,
}

impl AppState {
    pub fn new(cfg: &Args) -> Self {
        Self {
            devices: Default::default(),
            connected_device: None,
            song_dir: cfg.song_directory.clone(),
            last_song: None,
        }
    }

    pub fn query_songs(&self) -> std::io::Result<Vec<String>> {
        let mut songs = Vec::new();

        for song in std::fs::read_dir(&self.song_dir)? {
            if let Some(name) = song?
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".mid"))
            {
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
}

impl App {
    pub fn new(cfg: &Args) -> Self {
        Self {
            state: RwLock::new(AppState::new(cfg)),
            player: MidiPlayer::new(),
        }
    }

    pub fn new_shared(cfg: &Args) -> AppRef {
        Arc::new(App::new(cfg))
    }

    pub fn state(&self) -> impl std::ops::Deref<Target = AppState> + '_ {
        self.state.read().unwrap()
    }

    pub fn state_mut(&self) -> impl std::ops::DerefMut<Target = AppState> + '_ {
        self.state.write().unwrap()
    }

    pub fn query_songs(&self) -> std::io::Result<Vec<String>> {
        self.state.read().unwrap().query_songs()
    }

    pub fn store_song(&self, name: &str, events: Vec<RecordEvent>) -> std::io::Result<()> {
        self.state.write().unwrap().store_song(name, events)
    }

    pub async fn play_song(&self, name: String) -> std::io::Result<()> {
        self.player.stop().await;

        let (base_dir, device) = {
            let state = self.state.read().unwrap();
            (
                state.song_dir.clone(),
                state.connected_device.as_ref().map(|dev| dev.id()),
            )
        };

        let mut filepath = base_dir.join(&name);
        filepath.set_extension("mid");

        if let Some(dev) = device {
            info!("Playing {}", filepath.display());

            let reader = tokio::fs::File::open(filepath).await?;

            {
                let mut state = self.state.write().unwrap();
                state.last_song = Some(name);
            }
            self.player.play(dev, reader).await;

            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "No device for playing song",
            ))
        }
    }

    pub async fn stop_song(&self) {
        self.player.stop().await
    }

    pub fn poll_playing_song(&self) -> Option<String> {
        if self.player.is_playing() {
            self.state.read().unwrap().last_song.clone()
        } else {
            None
        }
    }
}
