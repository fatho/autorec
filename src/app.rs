use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use crate::{
    config::AppConfig,
    midi::{self, Device, DeviceInfo, MidiEvent, RecordEvent},
    player, recorder,
    store::{RecordingEntry, RecordingId, RecordingStore},
};

use color_eyre::eyre::bail;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info};

pub struct State {
    listening_device: Option<Device>,
    player_current: Option<RecordingId>,
    player_queued: Option<RecordingId>,
    player: Option<player::MidiPlayer>,
}

pub struct App {
    state: RwLock<State>,
    config: AppConfig,
    change_tx: broadcast::Sender<StateChange>,
    app_tx: mpsc::Sender<AppEvent>,
    midi: midi::Manager,
    store: RecordingStore,
}

impl App {
    pub async fn new(config: AppConfig) -> color_eyre::Result<Arc<Self>> {
        let (app_tx, app_rx) = mpsc::channel::<AppEvent>(16);
        let (change_tx, _) = broadcast::channel::<StateChange>(16);

        let store = RecordingStore::open(&config.data_directory).await?;

        let state = RwLock::new(State {
            listening_device: None,
            player_current: None,
            player_queued: None,
            player: None,
        });

        let midi = midi::Manager::new();
        let device_listener = midi.create_device_listener()?;

        let app = Arc::new(Self {
            state,
            config,
            change_tx,
            midi,
            app_tx,
            store,
        });

        // TODO: provide way to listen for failures of these threads
        tokio::spawn({
            let app = app.clone();
            async move { app_event_loop(app, app_rx).await }
        });
        tokio::spawn({
            let app = app.clone();
            async move { midi_event_loop(app, device_listener).await }
        });

        Ok(app)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StateChange> {
        self.change_tx.subscribe()
    }

    pub async fn query_recordings(&self) -> color_eyre::Result<Vec<RecordingEntry>> {
        self.store.get_recordings().await
    }

    pub async fn delete_recording(&self, recording: RecordingId) {
        let _ = self
            .app_tx
            .send(AppEvent::DeleteRecording { recording })
            .await;
    }

    pub async fn play_recording(&self, recording: RecordingId) {
        let _ = self.app_tx.send(AppEvent::PlayerStart { recording }).await;
        // TODO: await confirmation
    }

    pub async fn stop_playing(&self) {
        let _ = self.app_tx.send(AppEvent::PlayerRequestStop).await;
        // TODO: await confirmation
    }

    pub fn playing_recording(&self) -> Option<RecordingId> {
        let state = self.state.read().unwrap();
        state.player_current.clone()
    }

    pub async fn start_recording(&self) {
        let _ = self.app_tx.send(AppEvent::RecordingStart).await;
    }

    pub async fn finish_recording(&self, events: Vec<RecordEvent>) {
        let _ = self.app_tx.send(AppEvent::RecordingDone { events }).await;
    }

    fn notify(&self, change: StateChange) {
        // ignore errors - we don't care if no one is listening
        let _ = self.change_tx.send(change);
    }
}

async fn midi_event_loop(
    app: Arc<App>,
    mut listener: midi::DeviceListener,
) -> color_eyre::Result<()> {
    info!("Device listener started");
    loop {
        let event = listener.next().await?;

        let app_event = match event {
            midi::DeviceEvent::Connected { device, info } => {
                info!(
                    "Device {} ({}: {}) connected",
                    device.id(),
                    info.client_name,
                    info.port_name
                );
                AppEvent::DeviceConnected { device, info }
            }
            midi::DeviceEvent::Disconnected { device } => {
                info!("Device {} disconnected", device.id());
                AppEvent::DeviceDisconnected { device }
            }
        };

        // When the app stops listening, we can stop monitoring devices
        if app.app_tx.send(app_event).await.is_err() {
            info!("Device listener stopped");
            return Ok(());
        }
    }
}

async fn app_event_loop(app: Arc<App>, mut rx: mpsc::Receiver<AppEvent>) {
    while let Some(evt) = rx.recv().await {
        match evt {
            AppEvent::DeviceConnected { device, info } => {
                handle_new_device(&app, device, info);
            }
            AppEvent::DeviceDisconnected { .. } => {
                // Not particularly interesting for now
            }
            AppEvent::RecorderShutDown { .. } => {
                let mut state = app.state.write().unwrap();
                state.listening_device = None;
                app.notify(StateChange::ListenEnd);
            }
            AppEvent::RecordingStart => {
                app.notify(StateChange::RecordBegin);
            }
            AppEvent::RecordingDone { events } => {
                let name = chrono::Local::now().format("%Y%m%d-%H%M%S.mid").to_string();
                if let Err(err) = store_recording(&app, &name, events) {
                    error!("Failed to save song '{}': {}", name, err);
                    app.notify(StateChange::RecordError {
                        message: err.to_string(),
                    });
                    continue;
                }
                match app.store.insert_recording(Path::new(name.as_str())).await {
                    Ok(recording) => {
                        info!("Recorded song '{}' with id {}", name, recording.id.0);
                        app.notify(StateChange::RecordEnd { recording });
                    }
                    Err(err) => {
                        error!("Failed to register song '{}': {}", name, err);
                        app.notify(StateChange::RecordError {
                            message: err.to_string(),
                        });
                    }
                }
            }
            AppEvent::PlayerStart { recording } => {
                let playback_device = {
                    let mut state = app.state.write().unwrap();

                    if let Some(player) = state.player.as_mut() {
                        player.stop();
                        state.player_queued = Some(recording);
                        continue;
                    } else {
                        state.listening_device.as_ref().map(|dev| dev.id())
                    }
                };

                match play_recording(&app, playback_device, recording.clone()).await {
                    Ok(player) => {
                        app.notify(StateChange::PlayBegin {
                            recording: recording.clone(),
                        });

                        let mut state = app.state.write().unwrap();
                        state.player = Some(player);
                        state.player_current = Some(recording);
                    }
                    Err(err) => {
                        app.notify(StateChange::PlayError {
                            message: err.to_string(),
                        });
                        error!("Failed to play recording {}: {}", recording.0, err)
                    }
                }
            }
            AppEvent::PlayerRequestStop => {
                let state = app.state.read().unwrap();
                if let Some(player) = state.player.as_ref() {
                    player.stop();
                }
            }
            AppEvent::PlayerStopped => {
                let (playback_device, queued) = {
                    let mut state = app.state.write().unwrap();

                    (
                        state.listening_device.as_ref().map(|dev| dev.id()),
                        state.player_queued.take(),
                    )
                };
                if let Some(queued) = queued {
                    match play_recording(&app, playback_device, queued.clone()).await {
                        Ok(player) => {
                            app.notify(StateChange::PlayBegin {
                                recording: queued.clone(),
                            });

                            let mut state = app.state.write().unwrap();
                            state.player = Some(player);
                            state.player_current = Some(queued);
                        }
                        Err(err) => {
                            app.notify(StateChange::PlayError {
                                message: err.to_string(),
                            });
                            error!("Failed to play recording {}: {}", queued.0, err)
                        }
                    }
                } else {
                    let mut state = app.state.write().unwrap();
                    state.player_current = None;
                    state.player = None;
                    app.notify(StateChange::PlayEnd);
                }
            }
            AppEvent::DeleteRecording { recording } => {
                if let Err(err) = delete_recording(&app, recording).await {
                    error!("Failed to delete recording {}: {}", recording.0, err);
                }
            }
        }
    }
}

async fn delete_recording(app: &Arc<App>, id: RecordingId) -> color_eyre::Result<()> {
    let rec = app.store.get_recording_by_id(id).await?;
    app.store.delete_recording_by_id(id).await?;
    let rec_file = app.config.data_directory.join(&rec.filename);
    std::fs::remove_file(rec_file)?;
    app.notify(StateChange::RecordDelete { recording_id: id });
    Ok(())
}

async fn play_recording(
    app: &Arc<App>,
    device: Option<String>,
    recording: RecordingId,
) -> color_eyre::Result<player::MidiPlayer> {
    if let Some(output) = device {
        info!("Playing {}", recording.0);

        let entry = app.store.get_recording_by_id(recording).await?;

        let reader = open_recording(app, &entry.filename).await?;

        let (player, notifier) = player::MidiPlayer::new(output, Box::pin(reader)).await?;
        tokio::spawn({
            let app = app.clone();
            async move {
                let _ = notifier.await;
                let _ = app.app_tx.send(AppEvent::PlayerStopped).await;
            }
        });
        Ok(player)
    } else {
        bail!("No device for playing song")
    }
}

pub fn store_recording(app: &App, name: &str, events: Vec<RecordEvent>) -> std::io::Result<()> {
    let filepath = app.config.data_directory.join(name);

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

async fn open_recording(app: &Arc<App>, filename: &str) -> std::io::Result<tokio::fs::File> {
    let filepath = app.config.data_directory.join(&filename);

    tokio::fs::File::open(filepath).await
}

fn handle_new_device(app: &Arc<App>, device: Device, info: DeviceInfo) {
    let mut state = app.state.write().unwrap();

    if info.client_name.contains(&app.config.midi_device) {
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
            match app.midi.create_recorder(&device) {
                Ok(rec) => {
                    info!("Beginning recording on {}", device.id());
                    state.listening_device = Some(device.clone());
                    app.notify(StateChange::ListenBegin {
                        device: device.clone(),
                        info,
                    });

                    let inner_app = app.clone();
                    tokio::spawn(async move {
                        if let Err(err) = recorder::run_recorder(inner_app.clone(), rec).await {
                            error!("Recorder failed: {}", err)
                        } else {
                            info!("Recorder shut down");
                        }
                        // Notify app about stopping
                        let _ = inner_app
                            .app_tx
                            .send(AppEvent::RecorderShutDown { device })
                            .await;
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

/// Events controlling the application state.
pub enum AppEvent {
    DeviceConnected {
        device: Device,
        info: DeviceInfo,
    },
    DeviceDisconnected {
        device: Device,
    },
    RecorderShutDown {
        device: Device,
    },
    RecordingStart,
    RecordingDone {
        events: Vec<RecordEvent>,
    },
    DeleteRecording {
        recording: RecordingId,
    },
    /// Play the given recording (stops the playback of other recordings)
    PlayerStart {
        recording: RecordingId,
    },
    PlayerRequestStop,
    PlayerStopped,
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
    /// App starts playing back
    PlayBegin { recording: RecordingId },
    /// Failed to start playback
    PlayError { message: String },
    /// App stops playing back
    PlayEnd,
}
