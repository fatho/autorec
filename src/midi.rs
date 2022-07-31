// NOTE: Only supports Linux (via ALSA) at the moment

use alsa::seq::Addr;

mod alsa_backend;

#[derive(Debug, PartialEq, Eq, Hash, Clone)]
pub struct Device {
    client_id: i32,
    port_id: i32,
}

impl Device {
    pub fn id(&self) -> String {
        format!("{}:{}", self.client_id, self.port_id)
    }
}

impl From<alsa::seq::Addr> for Device {
    fn from(a: alsa::seq::Addr) -> Self {
        Self {
            client_id: a.client,
            port_id: a.port,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub client_name: String,
    pub port_name: String,
}

#[derive(Debug)]
pub enum DeviceEvent {
    Connected { device: Device, info: DeviceInfo },
    Disconnected { device: Device },
}

#[derive(Debug, Clone)]
pub struct RecordEvent {
    pub timestamp: u32,
    pub payload: MidiEvent,
}

#[derive(Debug, Clone)]
pub enum MidiEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        controller: u32,
        value: i32,
    },
    // TODO: do we need more?
}

#[derive(Debug)]
pub struct Manager {
    registry: alsa_backend::MidiRegistry,
}

impl Manager {
    pub fn new() -> Self {
        Self {
            registry: alsa_backend::MidiRegistry::new(),
        }
    }

    pub fn create_device_listener(&self) -> color_eyre::Result<DeviceListener> {
        alsa_backend::DeviceListener::new(&self.registry)
    }

    pub fn create_recorder(&self, source: &Device) -> color_eyre::Result<Recorder> {
        alsa_backend::MidiRecorder::new(
            &self.registry,
            Addr {
                client: source.client_id,
                port: source.port_id,
            },
        )
    }
}

pub type DeviceListener = alsa_backend::DeviceListener;
pub type Recorder = alsa_backend::MidiRecorder;

pub fn encode_midi_file(events: Vec<RecordEvent>) -> Vec<u8> {
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

    let mut midi_data = vec![];
    smf.write_std(&mut midi_data).expect("writing to vec doesn't fail");
    midi_data
}
