use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use color_eyre::Result;
use futures_util::future::ready;
use futures_util::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::task::JoinError;
use tracing::{debug, error, info};

mod midi;
mod recorder;
mod server;
mod state;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    // Set up midi context
    let mut midi = midi::MidiDeviceListener::new()?;
    let (device_event_sender, mut device_event_receiver) = tokio::sync::mpsc::channel(16);
    let midi_poll_thread = tokio::task::spawn_blocking(move || {
        info!("Started device polling");
        while !device_event_sender.is_closed() {
            let event = midi.wait_event(5000)?;

            if let Some(event) = event {
                let _ = device_event_sender.blocking_send(event);
            }
        }
        Ok(())
    });
    let midi_device_thread = tokio::spawn(async move {
        info!("Started device handling");
        while let Some(event) = device_event_receiver.recv().await {
            debug!("Got device event: {event:?}");
        }
        Ok(())
    });

    // Allow for graceful shutdowns (only catches SIGINT - not SIGTERM)
    let exit_signal = tokio::signal::ctrl_c();

    // Spawn a webserver for remote interaction
    let web_thread = tokio::spawn(async move {
        let app = Router::new().route("/status", get(status));

        let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
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
            info!("Terminated");
            Ok(())
        },
        thread_result = web_thread => handle_thread_exit("web", thread_result),
        thread_result = midi_poll_thread => handle_thread_exit("midi-poll", thread_result),
        thread_result = midi_device_thread => handle_thread_exit("midi-device", thread_result),
    }
}

// basic handler that responds with a static string
async fn status() -> &'static str {
    "Hello, World!"
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
