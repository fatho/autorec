use std::{
    path::Path,
    sync::{Arc},
};

use crate::{
    config::AppConfig,
    midi::{self, encode_midi_file, Device, DeviceInfo, RecordEvent},
    player::{self, MidiPlayQueue},
    recorder,
    store::{RecordingEntry, RecordingId, RecordingStore},
};

use color_eyre::eyre::bail;
use tokio::{
    sync::{broadcast, Mutex},
};
use tracing::{error, info};

#[derive(Debug)]
pub struct Shared {
    config: AppConfig,
    change_tx: broadcast::Sender<StateChange>,
    state: Mutex<State>,
}

#[derive(Debug)]
pub struct State {
    listening_device: Option<Device>,
    player: player::MidiPlayQueue<RecordingId>,
    midi: midi::Manager,
    store: RecordingStore,
    shutdown: broadcast::Sender<()>,
}

#[derive(Debug, Clone)]
pub struct App {
    shared: Arc<Shared>,
}

impl App {
    pub async fn new(config: AppConfig) -> color_eyre::Result<Self> {
        let (change_tx, _) = broadcast::channel::<StateChange>(16);

        let (shutdown, shutdown_rx) = broadcast::channel::<()>(1);

        let store = RecordingStore::open(&config.data_directory).await?;

        let midi = midi::Manager::new();
        let device_listener = midi.create_device_listener()?;
        let player = MidiPlayQueue::new();
        let player_events = player.subscribe();

        let state = State {
            listening_device: None,
            player,
            midi,
            store,
            shutdown,
        };

        let shared = Arc::new(Shared {
            config,
            change_tx,
            state: Mutex::new(state),
        });

        // TODO: provide way to listen for failures of this threads
        tokio::spawn({
            let shared = shared.clone();
            let shutdown_rx = shutdown_rx.resubscribe();
            async move { player_event_loop(shared, player_events, shutdown_rx).await }
        });
        tokio::spawn({
            let shared = shared.clone();
            async move { midi_event_loop(shared, device_listener, shutdown_rx).await }
        });

        Ok(App { shared })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.shared.change_tx.subscribe()
    }

    pub async fn query_recordings(&self) -> color_eyre::Result<Vec<RecordingEntry>> {
        let state = self.shared.state.lock().await;
        state.store.get_recordings().await
    }

    pub async fn delete_recording(&self, recording: RecordingId) -> color_eyre::Result<()> {
        let state = self.shared.state.lock().await;
        let rec = state.store.get_recording_by_id(recording).await?;
        state.store.delete_recording_by_id(recording).await?;
        let rec_file = self.shared.config.data_directory.join(&rec.filename);
        std::fs::remove_file(rec_file)?;
        self.shared.notify(StateChange::RecordDelete {
            recording_id: recording,
        });
        Ok(())
    }

    pub async fn rename_recording(&self, recording: RecordingId, new_name: String) -> color_eyre::Result<RecordingEntry> {
        let state = self.shared.state.lock().await;
        state.store.rename_recording_by_id(recording, new_name).await?;
        let rec = state.store.get_recording_by_id(recording).await?;
        self.shared.notify(StateChange::RecordUpdate { recording: rec.clone() });
        Ok(rec)
    }

    pub async fn play_recording(&self, recording: RecordingId) -> color_eyre::Result<()> {
        let mut state = self.shared.state.lock().await;
        if let Some(output) = state.listening_device.clone() {
            info!("Playing {}", recording.0);

            let entry = state.store.get_recording_by_id(recording).await?;

            let filepath = self.shared.config.data_directory.join(&entry.filename);

            let reader = tokio::fs::File::open(filepath).await?;

            state
                .player
                .play(recording, output.id(), Box::pin(reader))
                .await?;
            Ok(())
        } else {
            bail!("No device for playing song")
        }
    }

    pub async fn stop_playing(&self) {
        let mut state = self.shared.state.lock().await;
        state.player.stop().await
    }

    pub async fn playing_recording(&self) -> Option<RecordingId> {
        let state = self.shared.state.lock().await;
        state.player.current().await
    }
}

async fn player_event_loop(
    shared: Arc<Shared>,
    mut player_events: broadcast::Receiver<player::QueueEvent<RecordingId>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    loop {
        let evt = tokio::select! {
            _ = shutdown_rx.recv() => break,
            evt = player_events.recv() => evt
        };

        match evt {
            Ok(evt) => match evt {
                player::QueueEvent::PlaybackStart(recording) => {
                    shared.notify(StateChange::PlayBegin { recording })
                },
                player::QueueEvent::PlaybackStop(_) => {
                    shared.notify(StateChange::PlayEnd)
                },
            },
            Err(err) => match err {
                broadcast::error::RecvError::Closed => break,
                broadcast::error::RecvError::Lagged(_) => continue,
            },
        }
    }
}

impl Shared {
    fn notify(&self, change: StateChange) {
        // ignore errors - we don't care if no one is listening
        let _ = self.change_tx.send(change);
    }

    async fn handle_device_added(self: &Arc<Self>, device: Device, info: DeviceInfo) {
        let mut state = self.state.lock().await;

        if info.client_name.contains(&self.config.midi_device) {
            if let Some(dev) = state.listening_device.as_ref() {
                info!(
                    "New devices {} ({}) matches but already recording on {}",
                    device.id(),
                    info.client_name,
                    dev.id()
                );
            } else {
                info!("Matching client {} connected", info.client_name);
                // TODO: extract starting of recorder into its own function
                match state.midi.create_recorder(&device) {
                    Ok(rec) => {
                        info!("Beginning recording on {}", device.id());
                        state.listening_device = Some(device.clone());
                        self.notify(StateChange::ListenBegin {
                            device: device.clone(),
                            info,
                        });

                        let inner_shared = self.clone();
                        tokio::spawn(async move {
                            if let Err(err) =
                                recorder::run_recorder(inner_shared.clone(), rec).await
                            {
                                error!("Recorder failed: {}", err)
                            } else {
                                info!("Recorder shut down");
                            }
                            // Notify app about stopping
                            {
                                let mut state = inner_shared.state.lock().await;
                                state.listening_device = None;
                            }
                            inner_shared.notify(StateChange::ListenEnd);
                        });
                    }
                    Err(err) => {
                        error!("Failed to set up recorder for {}: {}", device.id(), err);
                    }
                }
            }
        } else {
            info!(
                "Ignoring client {} ({}): no match",
                device.id(),
                info.client_name
            );
        }
    }

    async fn handle_device_removed(self: &Arc<Self>, device: Device) {}

    pub(crate) async fn start_recording(&self) {
        self.notify(StateChange::RecordBegin);
    }

    pub(crate) async fn finish_recording(&self, events: Vec<RecordEvent>) {
        let state = self.state.lock().await;
        let name = chrono::Local::now().format("%Y%m%d-%H%M%S.mid").to_string();
        let data = encode_midi_file(events);
        match state
            .store
            .insert_recording(Path::new(name.as_str()), data)
            .await
        {
            Ok(recording) => {
                info!("Recorded song '{}' with id {}", name, recording.id.0);
                self.notify(StateChange::RecordEnd { recording });
            }
            Err(err) => {
                error!("Failed to register song '{}': {}", name, err);
                self.notify(StateChange::RecordError {
                    message: err.to_string(),
                });
            }
        }
    }
}

async fn midi_event_loop(
    shared: Arc<Shared>,
    mut listener: midi::DeviceListener,
    mut shutdown: broadcast::Receiver<()>,
) -> color_eyre::Result<()> {
    info!("Device listener started");
    loop {
        tokio::select! {
            _ = shutdown.recv() => {
                info!("Device listener stopped");
                break;
            }
            event = listener.next() => {
                match event? {
                    midi::DeviceEvent::Connected { device, info } => {
                        info!(
                            "Device {} ({}: {}) connected",
                            device.id(),
                            info.client_name,
                            info.port_name
                        );
                        shared.handle_device_added(device, info).await;
                    }
                    midi::DeviceEvent::Disconnected { device } => {
                        info!("Device {} disconnected", device.id());
                        shared.handle_device_removed(device).await;
                    }
                }

            }
        }
    }
    Ok(())
}

/// Events informing others about changes in the application state.
#[derive(Debug, Clone)]
pub enum StateChange {
    /// App begins listening the given MIDI device
    ListenBegin { device: Device, info: DeviceInfo },
    /// App stops listening MIDI device (usually because it was disconnected)
    ListenEnd,
    /// App starts recording
    RecordBegin,
    /// App stops recording (due to MIDI inactivity)
    RecordEnd { recording: RecordingEntry },
    /// Failed to record song
    RecordError { message: String },
    /// A recording was deleted
    RecordDelete { recording_id: RecordingId },
    /// A recording was updated
    RecordUpdate { recording: RecordingEntry },
    /// App starts playing back
    PlayBegin { recording: RecordingId },
    /// App stops playing back
    PlayEnd,
}
