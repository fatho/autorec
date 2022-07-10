use axum::{
    http::StatusCode,
    response::IntoResponse,
    routing::{get, get_service, post},
    Extension, Router,
};
use clap::Parser;
use color_eyre::{eyre::Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::task::JoinError;
use tower_http::{
    services::ServeDir,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{debug, error, info};

mod app;
mod args;
mod config;
mod midi;
mod player;
mod player2;
mod recorder;
mod server;
mod state;

/// Program to automatically start MIDI recordings of songs played on an attached MIDI device.
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Path of the config file
    #[clap(short('c'), long, default_value("autorec.toml"))]
    pub config: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    color_eyre::install()?;

    let args = Args::parse();
    let config_toml = std::fs::read_to_string(args.config).context("reading config file")?;
    let config = toml::from_str::<config::Config>(&config_toml).context("parsing config file")?;

    // Initialize state
    let app = app::App::new(config.app)?;

    // Allow for graceful shutdowns (only catches SIGINT - not SIGTERM)
    let exit_signal = tokio::signal::ctrl_c();

    // Spawn a web server for remote interaction
    let web_thread = tokio::spawn({
        let app = app.clone();
        async move {
            let mut router = Router::new()
                //.route("/devices", get(server::devices))
                .route("/songs", get(server::songs))
                .route("/play", post(server::play))
                .route("/stop", post(server::stop))
                .route("/play-status", get(server::play_status))
                .route("/updates", get(server::updates_ws))
                .route("/updates-sse", get(server::updates_sse));

            if let Some(dir) = config.web.serve_frontend.as_ref() {
                async fn handle_error(_err: std::io::Error) -> impl IntoResponse {
                    (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
                }

                router =
                    router.fallback(get_service(ServeDir::new(dir)).handle_error(handle_error));
            }

            router = router.layer(Extension(app)).layer(
                TraceLayer::new_for_http()
                    .make_span_with(DefaultMakeSpan::default().include_headers(true)),
            );

            let addr = SocketAddr::from(([0, 0, 0, 0], config.web.port));
            tracing::info!("Web server listening on http://{}", addr);
            axum::Server::bind(&addr)
                .serve(router.into_make_service())
                .await?;
            Result::<()>::Ok(())
        }
    });

    // Wait for the first to exit: this should normally be the signal handler
    // (unless something goes terribly wrong)
    tokio::select! {
        _ = exit_signal => {
            info!("Shutting down...");
            Ok(())
        },
        thread_result = web_thread => handle_thread_exit("web", thread_result),
        // TODO: poll App threads
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
