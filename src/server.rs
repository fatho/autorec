use axum::{Extension, Json, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::state::AppRef;

#[derive(Serialize, Deserialize)]
pub struct DeviceObject {
    id: String,
    description: String,
}


/// Return list of devices
pub async fn devices(app: Extension<AppRef>) -> Json<Vec<DeviceObject>> {
    let result = app
        .state()
        .devices
        .iter()
        .map(|(device, info)| DeviceObject {
            id: device.id(),
            description: format!("{} ({})", info.client_name, info.port_name),
        })
        .collect();

    Json(result)
}

/// Return list songs
pub async fn songs(app: Extension<AppRef>) -> Json<Vec<String>> {
    let mut songs = app.query_songs().unwrap_or_else(|err| {
        error!("Failed to list songs: {}", err);
        vec![]
    });
    songs.sort_by(|a, b| b.cmp(a));

    Json(songs)
}

#[derive(Serialize, Deserialize)]
pub struct PlayRequest {
    name: String,
}


#[derive(Serialize, Deserialize)]
pub struct AppError {
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.message).into_response()
    }
}

#[axum_macros::debug_handler]
pub async fn play(app: Extension<AppRef>, Json(request): Json<PlayRequest>) -> Result<Json<String>, AppError> {
    match app.play_song(request.name).await {
        Ok(()) => Ok(Json("ok".to_owned())),
        Err(err) => Err(AppError { message: err.to_string() }),
    }
}

#[axum_macros::debug_handler]
pub async fn stop(app: Extension<AppRef>, Json(()): Json<()>) -> Json<()> {
    app.stop_song().await;
    Json(())
}

pub async fn play_status(app: Extension<AppRef>) -> Json<Option<String>> {
    Json(app.poll_playing_song())
}
