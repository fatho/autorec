use std::{collections::HashMap, sync::Arc};

use crate::{
    config::AppConfig,
    midi::{self, encode_midi, Device, DeviceInfo, RecordEvent},
    player::{self, MidiPlayQueue},
    recorder,
    store::{RecordingId, RecordingInfo, RecordingStore},
};

use color_eyre::eyre::bail;
use tokio::sync::{broadcast, Mutex};
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
    #[allow(unused)]
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

    pub async fn query_recordings(&self) -> color_eyre::Result<Vec<RecordingInfo>> {
        let state = self.shared.state.lock().await;
        state.store.get_recording_infos().await
    }

    pub async fn delete_recording(&self, recording: RecordingId) -> color_eyre::Result<()> {
        let state = self.shared.state.lock().await;
        state.store.delete_recording_by_id(recording).await?;
        self.shared.notify(StateChange::RecordDelete {
            recording_id: recording,
        });
        Ok(())
    }

    pub async fn rename_recording(
        &self,
        recording: RecordingId,
        new_name: String,
    ) -> color_eyre::Result<RecordingInfo> {
        let state = self.shared.state.lock().await;
        state
            .store
            .rename_recording_by_id(recording, new_name)
            .await?;
        let rec = state.store.get_recording_info_by_id(recording).await?;
        self.shared.notify(StateChange::RecordUpdate {
            recording: rec.clone(),
        });
        Ok(rec)
    }

    pub async fn classify_recording(
        &self,
        recording: RecordingId,
    ) -> color_eyre::Result<Vec<(String, f64)>> {
        let state = self.shared.state.lock().await;

        // TODO: optimize

        fn update_histogram(midi_data: &[u8], hist: &mut [u32]) -> color_eyre::Result<()> {
            let midi = midly::Smf::parse(midi_data)?;
            if let Some(track) = midi.tracks.first() {
                for event in track {
                    if let midly::TrackEventKind::Midi {
                        message: midly::MidiMessage::NoteOn { key, .. },
                        ..
                    } = event.kind
                    {
                        hist[key.as_int() as usize] += 1;
                    }
                }
            }
            Ok(())
        }

        // Build histogram of queried recording
        let mut query_hist = vec![0; 128];
        let midi_data = state.store.get_recording_midi(recording).await?;
        update_histogram(&midi_data, &mut query_hist)?;

        // Build histogram per name
        let recs = state.store.get_recording_infos().await?;
        let mut groups: HashMap<&str, Vec<u32>> = HashMap::new();

        for rec in recs.iter() {
            if !rec.name.is_empty() && rec.id != recording {
                let group = groups
                    .entry(rec.name.as_str())
                    .or_insert_with(|| vec![0; 128]);

                let midi_data = state.store.get_recording_midi(rec.id).await?;
                update_histogram(&midi_data, group)?;
            }
        }

        // Compute cosine similarity for each name
        fn cosine_sim(a: &[u32], b: &[u32]) -> f64 {
            let mag_a = a.iter().map(|x| (x * x) as f64).sum::<f64>().sqrt();
            let mag_b = b.iter().map(|x| (x * x) as f64).sum::<f64>().sqrt();

            let dot = a
                .iter()
                .zip(b.iter())
                .map(|(x, y)| (x * y) as f64)
                .sum::<f64>();

            dot / (mag_a * mag_b)
        }

        let mut outcome = groups
            .iter()
            .filter_map(|(name, hist)| {
                Some((
                    *name,
                    ordered_float::NotNan::new(cosine_sim(&query_hist, hist)).ok()?,
                ))
            })
            .collect::<Vec<_>>();
        outcome.sort_by_key(|x| -x.1);

        Ok(outcome.into_iter().map(|x| (x.0.to_owned(), x.1.into_inner())).collect())
    }

    pub async fn play_recording(&self, recording: RecordingId) -> color_eyre::Result<()> {
        let mut state = self.shared.state.lock().await;
        if let Some(output) = state.listening_device.clone() {
            info!("Playing {}", recording.0);
            let data = state.store.get_recording_midi(recording).await?;

            state
                .player
                .play(recording, output.id(), Box::pin(std::io::Cursor::new(data)))
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
                }
                player::QueueEvent::PlaybackStop(_) => shared.notify(StateChange::PlayEnd),
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

    async fn handle_device_removed(self: &Arc<Self>, _device: Device) {}

    pub(crate) async fn start_recording(&self) {
        self.notify(StateChange::RecordBegin);
    }

    pub(crate) async fn finish_recording(&self, events: Vec<RecordEvent>) {
        let state = self.state.lock().await;
        let data = encode_midi(events);
        match state.store.insert_recording(data).await {
            Ok(recording) => {
                info!("Recording saved with id {}", recording.id.0);
                self.notify(StateChange::RecordEnd { recording });
            }
            Err(err) => {
                error!("Failed to store recording: {}", err);
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
    RecordEnd { recording: RecordingInfo },
    /// Failed to record song
    RecordError { message: String },
    /// A recording was deleted
    RecordDelete { recording_id: RecordingId },
    /// A recording was updated
    RecordUpdate { recording: RecordingInfo },
    /// App starts playing back
    PlayBegin { recording: RecordingId },
    /// App stops playing back
    PlayEnd,
}
