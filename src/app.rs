use std::sync::{Arc, Mutex, RwLock, RwLockWriteGuard};

use crate::{
    config::AppConfig,
    midi::{self, Device, DeviceInfo, RecordEvent}, recorder, player2,
};

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, info, error};

pub struct State {
    listening_device: Option<Device>,
    player_current: Option<RecordingId>,
    player_queued: Option<RecordingId>,
    player: Option<player2::MidiPlayer>,
}

pub struct App {
    state: RwLock<State>,
    config: AppConfig,
    change_tx: broadcast::Sender<StateChange>,
    app_tx: mpsc::Sender<AppEvent>,
    midi: midi::Manager,
}

impl App {
    pub fn new(config: AppConfig) -> color_eyre::Result<Arc<Self>> {
        let (app_tx, app_rx) = mpsc::channel::<AppEvent>(16);
        let (change_tx, _) = broadcast::channel::<StateChange>(16);

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

    pub fn query_songs(&self) -> std::io::Result<Vec<String>> {
        let mut songs = Vec::new();

        for song in std::fs::read_dir(&self.config.data_directory)? {
            if let Some(name) = song?
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".mid"))
            {
                songs.push(name.to_owned())
            }
        }

        Ok(songs)
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
            AppEvent::DeviceDisconnected { device } => {
                // Not particularly interesting for now
            }
            AppEvent::RecorderShutDown { device } => {
                let mut state = app.state.write().unwrap();
                state.listening_device = None;
                app.notify(StateChange::ListenEnd);
            }
            AppEvent::RecordingStart => {
                app.notify(StateChange::RecordBegin);
            }
            AppEvent::RecordingDone { events } => {
                app.notify(StateChange::RecordEnd { recording: todo!("store recording") });
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
                        app.notify(StateChange::PlayBegin { recording: recording.clone() });

                        let mut state = app.state.write().unwrap();
                        state.player = Some(player);
                        state.player_current = Some(recording);
                    },
                    // TODO: notify client about error somehow
                    Err(err) => error!("Failed to play recording {}: {}", recording.0, err),
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

                    (state.listening_device.as_ref().map(|dev| dev.id()), state.player_queued.take())
                };
                if let Some(queued) = queued {
                    match play_recording(&app, playback_device, queued.clone()).await {
                        Ok(player) => {
                            app.notify(StateChange::PlayBegin { recording: queued.clone() });

                            let mut state = app.state.write().unwrap();
                            state.player = Some(player);
                            state.player_current = Some(queued);
                        },
                        // TODO: notify client about error somehow
                        Err(err) => error!("Failed to play recording {}: {}", queued.0, err),
                    }
                } else {
                    let mut state = app.state.write().unwrap();
                    state.player_current = None;
                    state.player = None;
                    app.notify(StateChange::PlayEnd);
                }
            }
        }
    }
}

async fn play_recording(app: &Arc<App>, device: Option<String>, recording: RecordingId) -> std::io::Result<player2::MidiPlayer> {
    if let Some(output) = device {
        info!("Playing {}", recording.0);

        let reader = open_recording(app, &recording).await?;

        let (player, notifier) = player2::MidiPlayer::new(output, Box::pin(reader)).await?;
        tokio::spawn({
            let app = app.clone();
            async move {
                let _ = notifier.await;
                let _ = app.app_tx.send(AppEvent::PlayerStopped).await;
            }
        });
        Ok(player)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No device for playing song",
        ))
    }
}


async fn open_recording(app: &Arc<App>, recording: &RecordingId) -> std::io::Result<tokio::fs::File> {
    let mut filepath = app.config.data_directory.join(&recording.0);
    filepath.set_extension("mid");

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
                    app.notify(StateChange::ListenBegin { device: device.clone(), info });

                    let inner_app = app.clone();
                    tokio::spawn(async move {
                        let mut rec = rec;
                        while let Ok(Some(_)) = rec.next().await {
                        }
                        // TODO: reinstante recorder
                        // if let Err(err) =
                        //     recorder::run_recorder(todo!(), rec).await
                        // {
                        //     error!("Recorder failed: {}", err)
                        // } else {
                        //     info!("Recorder shut down");
                        // }
                        // Notify app about stopping
                        let _ = inner_app.app_tx.send(AppEvent::RecorderShutDown { device }).await;
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


#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct RecordingId(pub String);

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
    RecordEnd { recording: RecordingId },
    /// App starts playing back
    PlayBegin { recording: RecordingId },
    /// App stops playing back
    PlayEnd,
}
