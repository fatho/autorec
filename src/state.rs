use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    args::Args,
    midi::{Device, DeviceInfo, MidiEvent, RecordEvent},
};

pub type AppStateRef = Arc<Mutex<AppState>>;

#[derive(Default, Debug)]
pub struct AppState {
    pub devices: HashMap<Device, DeviceInfo>,
    pub song_dir: PathBuf,
}

impl AppState {
    pub fn new(cfg: &Args) -> Self {
        Self {
            devices: Default::default(),
            song_dir: cfg.song_directory.clone(),
        }
    }

    pub fn new_shared(cfg: &Args) -> AppStateRef {
        Arc::new(Mutex::new(AppState::new(cfg)))
    }

    pub fn store_song(&mut self, name: &str, events: Vec<RecordEvent>) -> Result<(), std::io::Error> {
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

pub struct RecorderState {
    pub device: Device,
}
