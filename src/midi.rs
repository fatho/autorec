// NOTE: Only supports Linux (via ALSA) at the moment

use std::{collections::VecDeque, ffi::CStr, os::unix::prelude::RawFd};

use alsa::seq::{Addr, PortCap, PortSubscribe, PortType};
use tokio::io::unix::AsyncFd;
use tracing::{debug, trace};

pub struct MidiDeviceListener {
    seq: alsa::Seq,
    #[allow(unused)]
    client: i32,
    #[allow(unused)]
    announce_port: i32,
    poll_fd: AsyncFd<RawFd>,
    event_buffer: VecDeque<DeviceEvent>,
}

#[derive(Debug)]
pub struct Port {
    client_id: i32,
    port_id: i32,
    client_name: Option<String>,
    port_name: Option<String>,
}

#[derive(Debug)]
pub enum DeviceEvent {
    Connected(Port),
    Disconnected(Port),
}

impl MidiDeviceListener {
    pub fn new() -> Result<Self, std::io::Error> {
        // Create ALSA client
        let seq = alsa::seq::Seq::open(None, None, true).map_err(helpers::alsa_io_err)?;
        let client = seq.client_id().map_err(helpers::alsa_io_err)?;
        seq.set_client_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-listener\0") })
            .map_err(helpers::alsa_io_err)?;

        // Create local port for receiving announcement events
        let announce_port = seq
            .create_simple_port(
                unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-announce\0") },
                PortCap::WRITE | PortCap::SUBS_WRITE,
                PortType::MIDI_GENERIC | PortType::APPLICATION,
            )
            .map_err(helpers::alsa_io_err)?;

        // Subscribe client via the local port to the global announcement port
        let subscribe = PortSubscribe::empty().map_err(helpers::alsa_io_err)?;
        subscribe.set_dest(Addr {
            client,
            port: announce_port,
        });
        subscribe.set_sender(Addr {
            client: helpers::SND_SEQ_CLIENT_SYSTEM,
            port: helpers::SND_SEQ_PORT_SYSTEM_ANNOUNCE,
        });
        seq.subscribe_port(&subscribe)
            .map_err(helpers::alsa_io_err)?;

        // Set up polling via tokio
        let fds = alsa::poll::Descriptors::get(&(&seq, Some(alsa::Direction::Capture)))
            .map_err(helpers::alsa_io_err)?;
        tracing::debug!("Sequencer fds {fds:?}");
        // Theoretically, there could be more FDs, but it seems that for Alsa Sequencers, the number
        // of file descriptors for polling is hard-coded to one.
        assert_eq!(fds.len(), 1);
        let poll_fd = AsyncFd::new(fds[0].fd)?;

        // Pre-generate "events" for devices that are already connected
        let event_buffer = helpers::alsa_ports(&seq)
            .into_iter()
            .map(DeviceEvent::Connected)
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
                trace!("returning buffered event");
                return Ok(event);
            }

            trace!("waiting for read readiness");
            let mut guard = self.poll_fd.readable().await?;

            let mut input = self.seq.input();

            loop {
                match input.event_input() {
                    Ok(event) => {
                        debug!("got event: {:?}", event.get_type());
                        match event.get_type() {
                            alsa::seq::EventType::PortExit => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    let port = helpers::port_from_addr(&self.seq, addr);
                                    self.event_buffer.push_back(DeviceEvent::Disconnected(port));
                                }
                            }
                            alsa::seq::EventType::PortStart => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    let port = helpers::port_from_addr(&self.seq, addr);
                                    self.event_buffer.push_back(DeviceEvent::Connected(port));
                                }
                            }
                            // Rest is uninteresting here
                            _ => {}
                        }
                    }
                    Err(err) if err.errno() == alsa::nix::errno::Errno::EAGAIN => {
                        trace!("events exhausted");
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
    pub fn is_port_usable(client: &ClientInfo, port: &PortInfo) -> bool {
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

    pub fn alsa_ports(seq: &alsa::seq::Seq) -> Vec<super::Port> {
        let mut ports = vec![];
        for client in alsa::seq::ClientIter::new(&seq) {
            let client_id = client.get_client();
            let client_name = client.get_name().ok().map(String::from);

            for port in alsa::seq::PortIter::new(&seq, client.get_client()) {
                if is_port_usable(&client, &port) {
                    ports.push(super::Port {
                        client_id,
                        client_name: client_name.clone(),
                        port_id: port.get_port(),
                        port_name: port.get_name().ok().map(String::from),
                    })
                }
            }
        }
        ports
    }

    pub fn port_from_addr(seq: &alsa::seq::Seq, addr: Addr) -> super::Port {
        super::Port {
            port_id: addr.port,
            client_id: addr.client,
            client_name: seq
                .get_any_client_info(addr.client)
                .and_then(|c| c.get_name().map(String::from))
                .ok(),
            port_name: seq
                .get_any_port_info(addr)
                .and_then(|p| p.get_name().map(String::from))
                .ok(),
        }
    }
}
