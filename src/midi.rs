// NOTE: Only supports Linux (via ALSA) at the moment

use std::{collections::VecDeque, ffi::CStr, os::unix::prelude::RawFd};

use alsa::seq::{Addr, PortCap, PortSubscribe, PortType, QueueTempo, PortInfo, EvNote, EventType};
use tokio::io::unix::AsyncFd;
use tracing::{debug, trace};

use helpers::alsa_io_err;

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

#[derive(Debug)]
pub struct DeviceInfo {
    pub client_name: String,
    pub port_name: String,
}

#[derive(Debug)]
pub enum DeviceEvent {
    Connected { device: Device, info: DeviceInfo },
    Disconnected { device: Device },
}

pub struct MidiDeviceListener {
    seq: alsa::Seq,
    #[allow(unused)]
    client: i32,
    #[allow(unused)]
    announce_port: i32,
    poll_fd: AsyncFd<RawFd>,
    event_buffer: VecDeque<DeviceEvent>,
}

impl MidiDeviceListener {
    pub fn new() -> Result<Self, std::io::Error> {
        // Create ALSA client
        let seq = alsa::seq::Seq::open(None, None, true).map_err(alsa_io_err)?;
        let client = seq.client_id().map_err(alsa_io_err)?;
        seq.set_client_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-listener\0") })
            .map_err(alsa_io_err)?;

        // Create local port for receiving announcement events
        let announce_port = seq
            .create_simple_port(
                unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-announce\0") },
                PortCap::WRITE | PortCap::SUBS_WRITE,
                PortType::MIDI_GENERIC | PortType::APPLICATION,
            )
            .map_err(alsa_io_err)?;

        // Subscribe client via the local port to the global announcement port
        let subscribe = PortSubscribe::empty().map_err(alsa_io_err)?;
        subscribe.set_dest(Addr {
            client,
            port: announce_port,
        });
        subscribe.set_sender(Addr {
            client: helpers::SND_SEQ_CLIENT_SYSTEM,
            port: helpers::SND_SEQ_PORT_SYSTEM_ANNOUNCE,
        });
        seq.subscribe_port(&subscribe)
            .map_err(alsa_io_err)?;

        // Set up polling via tokio
        let fds = alsa::poll::Descriptors::get(&(&seq, Some(alsa::Direction::Capture)))
            .map_err(alsa_io_err)?;
        tracing::debug!("Sequencer fds {fds:?}");
        // Theoretically, there could be more FDs, but it seems that for Alsa Sequencers, the number
        // of file descriptors for polling is hard-coded to one.
        assert_eq!(fds.len(), 1);
        let poll_fd = AsyncFd::new(fds[0].fd)?;

        // Pre-generate "events" for devices that are already connected
        let event_buffer = helpers::get_readable_midi_ports(&seq)
            .into_iter()
            .map(|(device, info)| DeviceEvent::Connected { device, info })
            .collect();

        Ok(Self {
            seq,
            client,
            announce_port,
            poll_fd,
            event_buffer,
        })
    }

    pub async fn next(&mut self) -> std::io::Result<DeviceEvent> {
        loop {
            if let Some(event) = self.event_buffer.pop_front() {
                trace!(client = self.client, "returning buffered event");
                return Ok(event);
            }

            trace!(client = self.client, "waiting for read readiness");
            let mut guard = self.poll_fd.readable().await?;

            let mut input = self.seq.input();

            loop {
                match input.event_input() {
                    Ok(event) => {
                        debug!(client = self.client, "got event: {:?}", event.get_type());
                        match event.get_type() {
                            EventType::PortStart => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    let info = helpers::get_device_info(&self.seq, addr);
                                    self.event_buffer.push_back(DeviceEvent::Connected {
                                        device: addr.into(),
                                        info,
                                    });
                                }
                            }
                            EventType::PortExit => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    self.event_buffer.push_back(DeviceEvent::Disconnected {
                                        device: addr.into(),
                                    });
                                }
                            }
                            // Rest is uninteresting here
                            _ => {}
                        }
                    }
                    Err(err) if err.errno() == alsa::nix::errno::Errno::EAGAIN => {
                        trace!(client = self.client, "events exhausted");
                        guard.clear_ready();
                        break;
                    }
                    Err(other) => return Err(other.errno().into()),
                }
            }
        }
    }
}


pub struct MidiRecorder {
    seq: alsa::Seq,
    #[allow(unused)]
    client: i32,
    #[allow(unused)]
    recv_port: i32,
    #[allow(unused)]
    recv_queue: i32,
    poll_fd: AsyncFd<RawFd>,
    event_buffer: VecDeque<RecordEvent>,
    record_device: Device,
}

#[derive(Debug, Clone)]
pub enum RecordEvent {
    NoteOn,
    NoteOff,
    ControlChange,
    // TODO: do we need more?
}

impl MidiRecorder {
    pub fn new(record_device: Device) -> Result<Self, std::io::Error> {
        // Create ALSA client
        let seq = alsa::seq::Seq::open(None, None, true).map_err(alsa_io_err)?;
        let client = seq.client_id().map_err(alsa_io_err)?;
        seq.set_client_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-recorder\0") })
            .map_err(alsa_io_err)?;

        debug!(client, "created sequencer");

        // Create queue for receiving events
        let recv_queue = seq.alloc_queue().map_err(alsa_io_err)?;

        debug!(client, "created queue {}", recv_queue);

        // These should be the defaults, but better to spell it out
        let tempo = QueueTempo::empty().map_err(alsa_io_err)?;
        tempo.set_ppq(96); // Pulses per Quarter note
        let bpm = 120;
        tempo.set_tempo(1000000 * 60 / bpm); // Microseconds per beat
        seq.set_queue_tempo(recv_queue, &tempo).map_err(alsa_io_err)?;

        debug!(client, "configured queue {}", recv_queue);

        // Create local port for receiving events
        let mut recv_port_info = PortInfo::empty().map_err(alsa_io_err)?;
        // Make it writable
        recv_port_info.set_capability(PortCap::WRITE | PortCap::SUBS_WRITE);
        recv_port_info.set_type(PortType::MIDI_GENERIC | PortType::APPLICATION);

        recv_port_info.set_midi_channels(16); // NOTE: does it matter? for now same as arecordmidi
        recv_port_info.set_name(
            unsafe { CStr::from_bytes_with_nul_unchecked(b"MIDI recording 1\0") },
        );

        // Enable timestamps for the events we receive
        recv_port_info.set_timestamp_queue(recv_queue);
        recv_port_info.set_timestamping(true);

         // Technically UB because `create_port` actually mutates the port-info.
        seq
            .create_port(&recv_port_info)
            .map_err(alsa_io_err)?;
        let recv_port = recv_port_info.get_port();

        debug!(client, "created port {}", recv_port);

        // Subscribe client via the local port to the global announcement port
        let subscribe = PortSubscribe::empty().map_err(alsa_io_err)?;
        subscribe.set_dest(Addr {
            client,
            port: recv_port,
        });
        subscribe.set_sender(Addr {
            client: record_device.client_id,
            port: record_device.port_id,
        });
        seq.subscribe_port(&subscribe)
            .map_err(alsa_io_err)?;

        debug!(client, "subcribed port to {}", record_device.id());

        // Set up polling via tokio
        let fds = alsa::poll::Descriptors::get(&(&seq, Some(alsa::Direction::Capture)))
            .map_err(alsa_io_err)?;
        tracing::debug!("Sequencer fds {fds:?}");
        // Theoretically, there could be more FDs, but it seems that for Alsa Sequencers, the number
        // of file descriptors for polling is hard-coded to one.
        assert_eq!(fds.len(), 1);
        let poll_fd = AsyncFd::new(fds[0].fd)?;

        Ok(Self {
            seq,
            client,
            recv_port,
            recv_queue,
            poll_fd,
            event_buffer: VecDeque::new(),
            record_device,
        })
    }

    pub async fn next(&mut self) -> std::io::Result<RecordEvent> {
        loop {
            if let Some(event) = self.event_buffer.pop_front() {
                trace!(client = self.client, "returning buffered event");
                return Ok(event);
            }

            trace!(client = self.client, "waiting for read readiness");
            let mut guard = self.poll_fd.readable().await?;

            let mut input = self.seq.input();

            loop {
                match input.event_input() {
                    Ok(event) => {
                        debug!(client = self.client, "got event: {:?} at {:?} note {:?}", event.get_type(), event.get_tick(), event.get_data::<EvNote>());
                        // NOTE: A "note off" event can either be sent as "note off", or as "note on" with a zero velocity
                        match event.get_type() {
                            EventType::Noteon => {
                                debug!("Note On");
                            }
                            EventType::Noteoff => {
                                debug!("Note off");
                            }
                            _ => {}
                        }
                    }
                    Err(err) if err.errno() == alsa::nix::errno::Errno::EAGAIN => {
                        trace!(client = self.client, "events exhausted");
                        guard.clear_ready();
                        break;
                    }
                    Err(other) => return Err(other.errno().into()),
                }
            }
        }
    }
}

mod helpers {
    use alsa::seq::{Addr, ClientInfo, PortCap, PortInfo, PortType};

    pub const SND_SEQ_CLIENT_SYSTEM: i32 = 0;
    pub const SND_SEQ_PORT_SYSTEM_ANNOUNCE: i32 = 1;

    /// Check whether the given port is suitable as a source for autorec.
    pub fn is_port_readable_midi(client: &ClientInfo, port: &PortInfo) -> bool {
        // Exclude system ports (timer & announce)
        client.get_client() != SND_SEQ_CLIENT_SYSTEM
            // Must support MIDI
            && port.get_type().contains(PortType::MIDI_GENERIC)
            // Must support reading and writing
            && port.get_capability().contains(
                PortCap::READ | PortCap::SUBS_READ
            )
    }

    pub fn alsa_io_err(err: alsa::Error) -> std::io::Error {
        err.errno().into()
    }

    pub fn get_readable_midi_ports(
        seq: &alsa::seq::Seq,
    ) -> Vec<(super::Device, super::DeviceInfo)> {
        let mut ports = vec![];
        for client in alsa::seq::ClientIter::new(&seq) {
            let client_id = client.get_client();
            let client_name = client
                .get_name()
                .ok()
                .map(String::from)
                .unwrap_or_else(String::new);
            for port in alsa::seq::PortIter::new(&seq, client.get_client()) {
                if is_port_readable_midi(&client, &port) {
                    let dev = super::Device {
                        client_id,
                        port_id: port.get_port(),
                    };
                    let info = super::DeviceInfo {
                        client_name: client_name.clone(),
                        port_name: port
                            .get_name()
                            .ok()
                            .map(String::from)
                            .unwrap_or_else(String::new),
                    };
                    ports.push((dev, info));
                }
            }
        }
        ports
    }

    pub fn get_device_info(seq: &alsa::seq::Seq, addr: Addr) -> super::DeviceInfo {
        super::DeviceInfo {
            client_name: seq
                .get_any_client_info(addr.client)
                .and_then(|c| c.get_name().map(String::from))
                .unwrap_or(String::new()),
            port_name: seq
                .get_any_port_info(addr)
                .and_then(|p| p.get_name().map(String::from))
                .unwrap_or(String::new()),
        }
    }
}
