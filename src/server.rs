use axum::{Extension, Json, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::state::AppStateRef;

#[derive(Serialize, Deserialize)]
pub struct DeviceObject {
    id: String,
    description: String,
}


/// Return list of devices
pub async fn devices(state_ref: Extension<AppStateRef>) -> Json<Vec<DeviceObject>> {
    let state = state_ref.lock().unwrap();

    let result = state
        .devices
        .iter()
        .map(|(device, info)| DeviceObject {
            id: device.id(),
            description: format!("{} ({})", info.client_name, info.port_name),
        })
        .collect();

    Json(result)
}

/// Return debug-printed state
pub async fn debug(state_ref: Extension<AppStateRef>) -> String {
    let state = state_ref.lock().unwrap();

    format!("{:#?}", state)
}


/// Return list songs
pub async fn songs(state_ref: Extension<AppStateRef>) -> Json<Vec<String>> {
    let state = state_ref.lock().unwrap();
    let mut songs = state.query_songs().unwrap_or_else(|err| {
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


pub async fn play(state_ref: Extension<AppStateRef>, Json(request): Json<PlayRequest>) -> Result<Json<String>, AppError> {
    let mut state = state_ref.lock().unwrap();

    match state.play_song(request.name) {
        Ok(()) => Ok(Json("ok".to_owned())),
        Err(err) => Err(AppError { message: err.to_string() }),
    }
}

pub async fn stop(state_ref: Extension<AppStateRef>, Json(()): Json<()>) -> Result<Json<String>, AppError> {
    let mut state = state_ref.lock().unwrap();

    match state.stop_song() {
        Ok(()) => Ok(Json("ok".to_owned())),
        Err(err) => Err(AppError { message: err.to_string() }),
    }
}

pub async fn play_status(state_ref: Extension<AppStateRef>) -> Result<Json<Option<String>>, AppError> {
    let mut state = state_ref.lock().unwrap();

    match state.poll_playing_song() {
        Ok(maybe_song) => Ok(Json(maybe_song)),
        Err(err) => Err(AppError { message: err.to_string() }),
    }
}
