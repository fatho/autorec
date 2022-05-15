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
use tokio_udev::{AsyncMonitorSocket, MonitorBuilder};
use tokio_util::task::LocalPoolHandle;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    // Local thread pool for udev because those types are !Send
    let pool = LocalPoolHandle::new(1);
    let udev_thread = spawn_udev_monitor(pool);

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

    tokio::select! {
        _ = exit_signal => {
            info!("Terminated");
            Ok(())
        },
        thread_result = udev_thread => handle_thread_exit("Udev", thread_result),
        thread_result = web_thread => handle_thread_exit("Web", thread_result),
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

fn spawn_udev_monitor(pool: LocalPoolHandle) -> tokio::task::JoinHandle<Result<()>> {
    let udev_thread = pool.spawn_pinned(|| async move {
        info!("Initializing udev");
        let builder = MonitorBuilder::new()?;
        // TODO: find out which device class the piano belongs to
        // .match_subsystem_devtype("usb", "usb_device")?;

        let monitor: AsyncMonitorSocket = builder.listen()?.try_into()?;

        info!("Waiting for events");

        monitor
            .for_each(|event| {
                match event {
                    Ok(event) => {
                        info!(
                            "Hotplug event: {}: {}",
                            event.event_type(),
                            event.device().syspath().display()
                        );
                    }
                    Err(err) => error!("Failed to get udev event {}", err),
                }
                ready(())
            })
            .await;

        Result::<()>::Ok(())
    });
    udev_thread
}
