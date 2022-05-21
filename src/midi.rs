// NOTE: Only supports Linux (via ALSA) at the moment

use std::{collections::VecDeque, ffi::CStr, os::unix::prelude::RawFd};

use alsa::seq::{Addr, PortCap, PortSubscribe, PortType};
use tokio::io::{unix::AsyncFd, Interest};
use tracing::{debug,trace};

pub struct MidiDeviceListener {
    seq: alsa::Seq,
    client: i32,
    announce_port: i32,
    fds: Box<[alsa::poll::pollfd]>,
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
        // TODO: change to non-blocking and figure out how to hook up with tokio
        let seq = alsa::seq::Seq::open(None, None, true).map_err(helpers::alsa_io_err)?;
        let client = seq.client_id().map_err(helpers::alsa_io_err)?;
        seq.set_client_name(unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-listener\0") })
            .map_err(helpers::alsa_io_err)?;

        let announce_port = seq
            .create_simple_port(
                unsafe { CStr::from_bytes_with_nul_unchecked(b"autorec-announce\0") },
                PortCap::WRITE | PortCap::SUBS_WRITE,
                PortType::MIDI_GENERIC | PortType::APPLICATION,
            )
            .map_err(helpers::alsa_io_err)?;

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

        let fds = alsa::poll::Descriptors::get(&(&seq, Some(alsa::Direction::Capture)))
            .map_err(helpers::alsa_io_err)?
            .into_boxed_slice();
        tracing::debug!("Sequencer FDs {fds:?}");

        let event_buffer = helpers::alsa_ports(&seq)
            .into_iter()
            .map(DeviceEvent::Connected)
            .collect();

        Ok(Self {
            seq,
            client,
            announce_port,
            fds,
            event_buffer,
        })
    }

    pub fn wait_event(&mut self, timeout_ms: i32) -> std::io::Result<Option<DeviceEvent>> {
        if let Some(event) = self.event_buffer.pop_front() {
            trace!("Returning buffered event");
            return Ok(Some(event));
        }

        trace!("Waiting for read readiness");

        // aseqdump also refreshes the pollfds every iteration
        alsa::poll::Descriptors::fill(&(&self.seq, Some(alsa::Direction::Capture)), &mut self.fds)
            .map_err(helpers::alsa_io_err)?;
        let ret = alsa::poll::poll(&mut self.fds, timeout_ms).map_err(helpers::alsa_io_err)?;

        trace!("Ready with {ret:?}");

        if ret == 0 {
            return Ok(None);
        }

        let mut input = self.seq.input();

        loop {
            match input.event_input() {
                Ok(event) => {
                    debug!("Got event: {:?}", event.get_type());
                    match event.get_type() {
                        alsa::seq::EventType::PortExit => {
                            if let Some(addr) = event.get_data::<Addr>() {
                                let port = helpers::port_from_addr(&self.seq, addr);
                                self.event_buffer.push_back(DeviceEvent::Disconnected(port));
                            }
                        }
                        alsa::seq::EventType::PortStart => {
                            // TODO: figure out why ClientStart happens immediately, but PortStart
                            // only when running `aseqdump -l` for listing all the ports. Are ports lazy?
                            if let Some(addr) = event.get_data::<Addr>() {
                                let port = helpers::port_from_addr(&self.seq, addr);
                                self.event_buffer.push_back(DeviceEvent::Connected(port));
                            }
                        }
                        alsa::seq::EventType::ClientStart => {
                            if let Some(addr) = event.get_data::<Addr>() {
                                trace!("new client: {}", addr.client);
                            }
                        }
                        // Rest is uninteresting here
                        _ => {},
                    }
                }
                Err(err) if err.errno() == alsa::nix::errno::Errno::EWOULDBLOCK => {
                    trace!("no more events for now");
                    break;
                }
                Err(other) => return Err(other.errno().into()),
            }
        }

        Ok(self.event_buffer.pop_front())
    }
}

// IDEA:
//   - enumerate existing ports up front
//   - send "fake" events from existing ports before resuming real events

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
