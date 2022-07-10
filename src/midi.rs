// NOTE: Only supports Linux (via ALSA) at the moment

use alsa::seq::{
    Addr
};

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

pub struct Manager {
    registry: alsa_backend::MidiRegistry,
}

impl Manager {
    pub fn new() -> Self {
        Self { registry: alsa_backend::MidiRegistry::new() }
    }

    pub fn create_device_listener(&self) -> color_eyre::Result<DeviceListener> {
        alsa_backend::DeviceListener::new(&self.registry)
    }

    pub fn create_recorder(&self, source: &Device) -> color_eyre::Result<Recorder> {
        alsa_backend::MidiRecorder::new(&self.registry, Addr {
            client: source.client_id,
            port: source.port_id,
        })
    }
}

pub type DeviceListener = alsa_backend::DeviceListener;
pub type Recorder = alsa_backend::MidiRecorder;
