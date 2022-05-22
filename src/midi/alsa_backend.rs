use std::{
    collections::{HashSet, VecDeque},
    ffi::{CStr, CString},
    os::unix::prelude::RawFd,
    sync::{Arc, Mutex},
};

use alsa::seq::{
    Addr, EvCtrl, EvNote, EventType, PortCap, PortInfo, PortSubscribe, PortType, QueueTempo,
};
use tokio::io::unix::AsyncFd;
use tracing::{debug, trace, warn};

use super::{DeviceEvent, RecordEvent, RecordPayload};

/// There should only be one instance of this.
#[derive(Debug, Clone)]
pub struct MidiRegistry {
    // std Mutex since we're only protecting data
    data: Arc<Mutex<Data>>,
}

#[derive(Debug, Default)]
struct Data {
    /// Set of clients that were create from this registry
    clients: HashSet<i32>,
}

impl MidiRegistry {
    pub fn new() -> Self {
        Self {
            data: Arc::new(Mutex::new(Data::default())),
        }
    }

    pub fn new_client(&self, name: &str) -> color_eyre::Result<Client> {
        let mut data = self.data.lock().expect("mutex poisoned");

        // Create ALSA client
        let seq = alsa::seq::Seq::open(None, None, true)?;
        let client_id = seq.client_id()?;
        let cname: CString = CString::new(name)?;
        seq.set_client_name(&cname)?;

        data.clients.insert(client_id);

        Ok(Client {
            seq,
            id: client_id,
            registry: self.clone(),
        })
    }

    fn drop_client(&self, client: i32) {
        let mut data = self.data.lock().expect("mutex poisoned");
        data.clients.remove(&client);
    }

    fn is_known_client(&self, client: i32) -> bool {
        let data = self.data.lock().expect("mutex poisoned");
        data.clients.contains(&client)
    }
}

pub struct Client {
    seq: alsa::seq::Seq,
    id: i32,
    registry: MidiRegistry,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.registry.drop_client(self.id);
    }
}

pub struct EventsPoll<E> {
    client: Client,
    poll_fd: AsyncFd<RawFd>,
    event_buffer: VecDeque<E>,
}

impl<E> EventsPoll<E> {
    pub fn new(client: Client) -> color_eyre::Result<Self> {
        // Set up polling via tokio
        let fds = alsa::poll::Descriptors::get(&(&client.seq, Some(alsa::Direction::Capture)))?;
        tracing::debug!("Sequencer fds {fds:?}");
        // Theoretically, there could be more FDs, but it seems that for Alsa Sequencers, the number
        // of file descriptors for polling is hard-coded to one.
        assert_eq!(fds.len(), 1);
        let poll_fd = AsyncFd::new(fds[0].fd)?;

        Ok(Self {
            client,
            poll_fd,
            event_buffer: VecDeque::new(),
        })
    }

    pub async fn next(
        &mut self,
        mut process: impl FnMut(&alsa::seq::Event) -> Option<E>,
    ) -> color_eyre::Result<E> {
        loop {
            if let Some(event) = self.event_buffer.pop_front() {
                trace!(client = self.client.id, "returning buffered event");
                return Ok(event);
            }

            trace!(client = self.client.id, "waiting for read readiness");
            let mut guard = self.poll_fd.readable().await?;

            let mut input = self.client.seq.input();

            loop {
                match input.event_input() {
                    Ok(event) => {
                        trace!(
                            client = self.client.id,
                            "got event: {:?} at {:?}",
                            event.get_type(),
                            event.get_tick(),
                        );
                        if let Some(event) = process(&event) {
                            self.event_buffer.push_back(event);
                        }
                    }
                    Err(err) if err.errno() == alsa::nix::errno::Errno::EAGAIN => {
                        trace!(client = self.client.id, "events exhausted");
                        guard.clear_ready();
                        break;
                    }
                    Err(other) => return Err(other.errno().into()),
                }
            }
        }
    }
}

enum AlsaDeviceEvent {
    PortConnected { addr: Addr },
    PortDisconnected { addr: Addr },
}

pub struct DeviceListener {
    poll: EventsPoll<AlsaDeviceEvent>,
    active: HashSet<Addr>,
}

impl DeviceListener {
    pub fn new(registry: &MidiRegistry) -> color_eyre::Result<Self> {
        let client = registry.new_client("autorec-listener")?;

        // Create local port for receiving announcement events
        let announce_port = client.seq.create_simple_port(
            unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-announce\0") },
            PortCap::WRITE | PortCap::SUBS_WRITE,
            PortType::MIDI_GENERIC | PortType::APPLICATION,
        )?;

        // Subscribe client via the local port to the global announcement port
        let subscribe = PortSubscribe::empty()?;
        subscribe.set_dest(Addr {
            client: client.id,
            port: announce_port,
        });
        subscribe.set_sender(Addr {
            client: internal::SND_SEQ_CLIENT_SYSTEM,
            port: internal::SND_SEQ_PORT_SYSTEM_ANNOUNCE,
        });
        client.seq.subscribe_port(&subscribe)?;

        // Set up polling
        let mut poll = EventsPoll::new(client)?;

        // Pre-generate "events" for devices that are already connected
        for addr in internal::get_readable_midi_ports(&poll.client.seq) {
            // Filter internal clients
            if !registry.is_known_client(addr.client) {
                poll.event_buffer
                    .push_back(AlsaDeviceEvent::PortConnected { addr })
            }
        }

        Ok(Self {
            poll,
            active: HashSet::new(),
        })
    }

    pub async fn next(&mut self) -> color_eyre::Result<DeviceEvent> {
        loop {
            let alsa_event = self
                .poll
                .next(move |event| {
                    match event.get_type() {
                        EventType::PortStart => {
                            let addr = event.get_data::<Addr>().expect("must have addr");
                            Some(AlsaDeviceEvent::PortConnected { addr })
                        }
                        EventType::PortExit => {
                            let addr = event.get_data::<Addr>().expect("must have addr");
                            Some(AlsaDeviceEvent::PortDisconnected { addr })
                        }
                        // Rest is uninteresting here
                        _ => None,
                    }
                })
                .await?;

            match alsa_event {
                AlsaDeviceEvent::PortConnected { addr } => {
                    if !self.poll.client.registry.is_known_client(addr.client)
                        && internal::is_port_readable_midi_addr(&self.poll.client.seq, addr)
                    {
                        let info = internal::get_device_info(&self.poll.client.seq, addr);
                        if !self.active.insert(addr) {
                            warn!("duplicate PortConnected for {:?}", addr)
                        }
                        return Ok(DeviceEvent::Connected {
                            device: addr.into(),
                            info,
                        });
                    } else {
                        continue;
                    }
                }
                AlsaDeviceEvent::PortDisconnected { addr } => {
                    if self.active.remove(&addr) {
                        return Ok(DeviceEvent::Disconnected {
                            device: addr.into(),
                        });
                    } else {
                        // Skip event - since we also didn't send the corresponding connect event
                        // apparently
                        continue;
                    }
                }
            }
        }
    }
}

pub struct MidiRecorder {
    poll: Option<EventsPoll<RecordEvent>>,
    last_tick: u32,
    bpm: u32,
    ppq: i32,
}

impl MidiRecorder {
    pub fn new(registry: &MidiRegistry, source: Addr) -> color_eyre::Result<Self> {
        let client = registry.new_client("autorec-listener")?;

        // Create queue for receiving events
        let recv_queue = client.seq.alloc_queue()?;

        debug!(client = client.id, "created queue {}", recv_queue);

        // These should be the defaults, but better to spell it out
        let tempo = QueueTempo::empty()?;
        let bpm = 120;
        let ppq = 96;
        tempo.set_ppq(ppq); // Pulses per Quarter note
        tempo.set_tempo(1000000 * 60 / bpm); // Microseconds per beat
        client.seq.set_queue_tempo(recv_queue, &tempo)?;

        debug!(client = client.id, "configured queue {}", recv_queue);

        // Create local port for receiving events
        let mut recv_port_info = PortInfo::empty()?;
        // Make it writable
        recv_port_info.set_capability(PortCap::WRITE | PortCap::SUBS_WRITE);
        recv_port_info.set_type(PortType::MIDI_GENERIC | PortType::APPLICATION);

        recv_port_info.set_midi_channels(16); // NOTE: does it matter? for now same as arecordmidi
        recv_port_info
            .set_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"MIDI recording 1\0") });

        // Enable timestamps for the events we receive
        recv_port_info.set_timestamp_queue(recv_queue);
        recv_port_info.set_timestamping(true);

        client.seq.create_port(&recv_port_info)?;
        let recv_port = recv_port_info.get_port();

        debug!(client = client.id, "created port {}", recv_port);

        // Subscribe client via the local port to the global announcement port
        let subscribe = PortSubscribe::empty()?;
        subscribe.set_dest(Addr {
            client: client.id,
            port: recv_port,
        });
        subscribe.set_sender(source);
        subscribe.set_queue(recv_queue);
        subscribe.set_time_update(true);
        client.seq.subscribe_port(&subscribe)?;

        debug!(
            client = client.id,
            "subcribed port to {}:{}", source.client, source.port
        );

        // Start the queue
        debug!(client = client.id, recv_queue, "starting queue");
        client
            .seq
            .control_queue(recv_queue, EventType::Start, 0, None)?;
        client.seq.drain_output()?; // flush

        // Set up polling
        let poll = EventsPoll::new(client)?;

        Ok(Self {
            poll: Some(poll),
            last_tick: 0,
            bpm,
            ppq,
        })
    }

    pub fn tick_to_duration(&self, tick: u32) -> std::time::Duration {
        std::time::Duration::from_micros(
            (tick as u64) * 1000000 * 60 / (self.bpm as u64 * self.ppq as u64)
        )
    }

    pub async fn next(&mut self) -> color_eyre::Result<RecordEvent> {
        if let Some(poll) = self.poll.as_mut() {
            let alsa_event = poll
                .next(|event| {
                    let tick = event.get_tick().expect("should have tick");

                    let payload = match event.get_type() {
                        EventType::Noteon => {
                            let note = event.get_data::<EvNote>().expect("must have note data");
                            // NOTE: A "note off" event can either be sent as "note off", or as
                            // "note on" with a zero velocity
                            if note.velocity > 0 {
                                Some(RecordPayload::NoteOn {
                                    channel: note.channel,
                                    note: note.note,
                                    velocity: note.velocity,
                                })
                            } else {
                                Some(RecordPayload::NoteOff {
                                    channel: note.channel,
                                    note: note.note,
                                })
                            }
                        }
                        EventType::Noteoff => {
                            let note = event.get_data::<EvNote>().expect("must have note data");
                            Some(RecordPayload::NoteOff {
                                channel: note.channel,
                                note: note.note,
                            })
                        }
                        EventType::Controller => {
                            let ctrl = event
                                .get_data::<EvCtrl>()
                                .expect("must have controller data");
                            Some(RecordPayload::ControlChange {
                                channel: ctrl.channel,
                                controller: ctrl.param,
                                value: ctrl.value,
                            })
                        }
                        EventType::PortUnsubscribed => {
                            // No need to check which port as we only subscribed to one
                            Some(RecordPayload::RecordEnd)
                        }
                        _ => None,
                    };
                    payload.map(|payload| {
                        RecordEvent {
                            timestamp: tick, // TODO: handle tick overflow?
                            payload,
                        }
                    })
                })
                .await?;

            self.last_tick = alsa_event.timestamp;

            if let RecordPayload::RecordEnd = alsa_event.payload {
                self.poll = None;
            }
            Ok(alsa_event)
        } else {
            panic!("called next after recording ended")
        }
    }
}

mod internal {
    use alsa::seq::{Addr, ClientInfo, PortCap, PortInfo, PortType};

    use crate::midi::{DeviceInfo};

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

    /// Check whether the given port is suitable as a source for autorec.
    pub fn is_port_readable_midi_addr(seq: &alsa::seq::Seq, addr: Addr) -> bool {
        if let Some(client) = seq.get_any_client_info(addr.client).ok() {
            if let Some(port) = seq.get_any_port_info(addr).ok() {
                return is_port_readable_midi(&client, &port);
            }
        }
        false
    }

    pub fn get_readable_midi_ports(seq: &alsa::seq::Seq) -> impl Iterator<Item = Addr> + '_ {
        alsa::seq::ClientIter::new(&seq).flat_map(move |client| {
            let client_id = client.get_client();

            alsa::seq::PortIter::new(&seq, client_id).filter_map(move |port| {
                if is_port_readable_midi(&client, &port) {
                    Some(Addr {
                        client: client_id,
                        port: port.get_port(),
                    })
                } else {
                    None
                }
            })
        })
    }

    pub fn get_device_info(seq: &alsa::seq::Seq, addr: Addr) -> DeviceInfo {
        DeviceInfo {
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
