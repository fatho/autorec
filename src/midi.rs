// NOTE: Only supports Linux (via ALSA) at the moment

use std::{ffi::CStr, os::unix::prelude::RawFd};

use alsa::seq::{Addr, PortCap, PortSubscribe, PortType};
use tokio::io::unix::AsyncFd;

use crate::midi::helpers::alsa_ports;

pub struct MidiDeviceListener {
    seq: alsa::Seq,
    client: i32,
    announce_port: i32,
    async_fds: Vec<AsyncFd<RawFd>>,
    event_buffer: Vec<DeviceEvent>,
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
            .map_err(helpers::alsa_io_err)?;
        let async_fds = fds
            .iter()
            .map(|poll_fd| AsyncFd::new(poll_fd.fd as RawFd))
            .collect::<Result<Vec<_>, _>>()?;

        tracing::info!("{fds:?}");

        let event_buffer = alsa_ports(&seq)
            .into_iter()
            .map(DeviceEvent::Connected)
            .collect();

        Ok(Self {
            seq,
            client,
            announce_port,
            async_fds,
            event_buffer,
        })
    }

    pub async fn listen(&mut self) -> std::io::Result<DeviceEvent> {
        'outer: loop {
            if let Some(event) = self.event_buffer.pop() {
                return Ok(event);
            }

            let mut guards = vec![];
            for fd in self.async_fds.iter() {
                let guard = fd.readable().await;
                guards.push(guard)
            }

            let mut input = self.seq.input();

            loop {
                match input.event_input() {
                    Ok(event) => {
                        match event.get_type() {
                            alsa::seq::EventType::PortExit => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    let port = helpers::port_from_addr(&self.seq, addr)
                                        .map_err(helpers::alsa_io_err)?;
                                    self.event_buffer.push(DeviceEvent::Disconnected(port));
                                }
                            }
                            alsa::seq::EventType::PortStart => {
                                if let Some(addr) = event.get_data::<Addr>() {
                                    let port = helpers::port_from_addr(&self.seq, addr)
                                        .map_err(helpers::alsa_io_err)?;
                                    self.event_buffer.push(DeviceEvent::Connected(port));
                                }
                            },
                            // Rest is uninteresting here
                            _ => (),
                        }
                    }
                    Err(err) if err.errno() == alsa::nix::errno::Errno::EWOULDBLOCK => {
                        continue 'outer
                    }
                    Err(other) => return Err(other.errno().into()),
                }
            }
        }
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

    pub fn port_from_addr(seq: &alsa::seq::Seq, addr: Addr) -> Result<super::Port, alsa::Error> {
        let port_info = seq.get_any_port_info(addr)?;
        let client_info = seq.get_any_client_info(addr.client)?;
        Ok(super::Port {
            port_id: addr.port,
            client_id: addr.client,
            client_name: client_info.get_name().ok().map(String::from),
            port_name: port_info.get_name().ok().map(String::from),
        })
    }
}
