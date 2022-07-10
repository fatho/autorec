use std::sync::Arc;

use axum::{Extension, Json, response::IntoResponse, http::StatusCode};
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::app::{App, RecordingId};

#[derive(Serialize, Deserialize)]
pub struct DeviceObject {
    id: String,
    description: String,
}


// TODO: reinstate /devices endpoint
// /// Return list of devices
// pub async fn devices(app: Extension<Arc<App>>) -> Json<Vec<DeviceObject>> {
//     let result = app
//         .state()
//         .devices
//         .iter()
//         .map(|(device, info)| DeviceObject {
//             id: device.id(),
//             description: format!("{} ({})", info.client_name, info.port_name),
//         })
//         .collect();

//     Json(result)
// }

/// Return list songs
pub async fn songs(app: Extension<Arc<App>>) -> Json<Vec<String>> {
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
pub async fn play(app: Extension<Arc<App>>, Json(request): Json<PlayRequest>) -> Result<Json<()>, AppError> {
    app.play_recording(RecordingId(request.name)).await;
    Ok(Json(()))
}

#[axum_macros::debug_handler]
pub async fn stop(app: Extension<Arc<App>>, Json(()): Json<()>) -> Json<()> {
    app.stop_playing().await;
    Json(())
}

pub async fn play_status(app: Extension<Arc<App>>) -> Json<Option<String>> {
    Json(app.playing_recording().map(|rec| rec.0))
}
