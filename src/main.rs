use axum::{
    routing::{get, post},
    Extension, Router,
};
use clap::Parser;
use color_eyre::Result;
use state::AppState;
use std::{net::SocketAddr};
use tokio::task::JoinError;
use tracing::{debug, error, info};

mod args;
mod midi;
mod recorder;
mod server;
mod state;


#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    let args = args::Args::parse();

    // Initialize state
    let proto_state_ref = AppState::new_shared(&args);

    // Set up midi context
    let midi = midi::Manager::new();

    let mut devices = midi.create_device_listener()?;

    let state_ref = proto_state_ref.clone();
    let midi_device_thread = tokio::spawn(async move {
        info!("Started device handling");
        loop {
            let event = devices.next().await?;
            debug!("Got device event: {event:?}");

            match event {
                midi::DeviceEvent::Connected { device, info } => {
                    let mut state = state_ref.lock().unwrap();

                    if info.client_name.contains(&args.midi_client) {
                        if let Some(dev) = state.connected_device.as_ref() {
                            info!("Already recording on {}", dev.id());
                        } else {
                            info!("Matching client {} connected", info.client_name);
                            state.connected_device = Some(device.clone());
                            match midi.create_recorder(&device) {
                                Ok(rec) => {
                                    let inner_state_ref = state_ref.clone();
                                    tokio::spawn(async move {
                                        info!("Beginning recording");
                                        if let Err(err) = recorder::run_recorder(inner_state_ref.clone(), rec).await {
                                            error!("Recorder failed: {}", err)
                                        } else {
                                            info!("Recorder shut down");
                                        }
                                        // reset recording state
                                        let mut state = inner_state_ref.lock().unwrap();
                                        state.connected_device = None;
                                    });
                                }
                                Err(err) => {
                                    error!("Failed to set up recorder for {}: {}", device.id(), err);
                                }
                            }
                        }
                    } else {
                        info!("Ignoring client {}: no match", info.client_name);
                    }

                    state.devices.insert(device, info);
                }
                midi::DeviceEvent::Disconnected { device } => {
                    let mut state = state_ref.lock().unwrap();
                    state.devices.remove(&device);
                }
            }
        }
    });

    // Allow for graceful shutdowns (only catches SIGINT - not SIGTERM)
    let exit_signal = tokio::signal::ctrl_c();

    // Spawn a web server for remote interaction
    let state_ref = proto_state_ref.clone();
    let web_thread = tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(server::startpage))
            .route("/devices", get(server::devices))
            .route("/debug", get(server::debug))
            .route("/songs", get(server::songs))
            .route("/play", post(server::play))
            .route("/stop", post(server::stop))
            .layer(Extension(state_ref));

        let addr = SocketAddr::from(([0, 0, 0, 0], 8000));
        tracing::info!("Web server listening on http://{}", addr);
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await?;
        Result::<()>::Ok(())
    });

    // Wait for the first to exit: this should normally be the signal handler
    // (unless something goes terribly wrong)
    tokio::select! {
        _ = exit_signal => {
            info!("Shutting down...");
            Ok(())
        },
        thread_result = web_thread => handle_thread_exit("web", thread_result),
        thread_result = midi_device_thread => handle_thread_exit("midi-device", thread_result),
    }
}

fn handle_thread_exit(
    thread_name: &'static str,
    thread_result: Result<Result<()>, JoinError>,
) -> Result<()> {
    match thread_result {
        Ok(inner) => {
            error!("Thread '{thread_name}' failed");
            inner
        }
        Err(join_err) if join_err.is_panic() => {
            error!("Thread '{thread_name}' panicked");
            // Resume the panic on the main task
            std::panic::resume_unwind(join_err.into_panic());
        }
        Err(join_err) if join_err.is_cancelled() => {
            info!("Thread '{thread_name}' was cancelled");
            Ok(())
        }
        Err(join_err) => {
            error!("Thread '{thread_name}' failed for an unknown reason");
            Err(join_err.into())
        }
    }
}
