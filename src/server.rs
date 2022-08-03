use std::convert::Infallible;

use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Sse,
    },
    Extension, Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio_stream::StreamExt;
use tracing::error;

use crate::{
    app::{App, StateChange},
    store::{RecordingId, RecordingInfo},
};

#[derive(Serialize, Deserialize)]
pub struct DeviceObject {
    id: String,
    description: String,
}

// TODO: reinstate /devices endpoint
// /// Return list of devices
// pub async fn devices(app: Extension<App>) -> Json<Vec<DeviceObject>> {
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

#[derive(Serialize)]
pub struct RecInfo {
    pub id: RecordingId,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl From<RecordingInfo> for RecInfo {
    fn from(entry: RecordingInfo) -> Self {
        RecInfo {
            id: entry.id,
            name: entry.name.clone(),
            created_at: entry.created_at,
        }
    }
}

/// Return list of recordings
pub async fn get_recordings(app: Extension<App>) -> Json<Vec<RecInfo>> {
    let songs = app.query_recordings().await.map_or_else(
        |err| {
            error!("Failed to list songs: {}", err);
            vec![]
        },
        |songs| songs.into_iter().map(RecInfo::from).collect(),
    );

    Json(songs)
}

/// Delete a recording
pub async fn delete_recording(
    app: Extension<App>,
    Path((recording_id,)): Path<(RecordingId,)>,
) -> Result<Json<()>, AppError> {
    app.delete_recording(recording_id).await?;
    Ok(Json(()))
}

#[derive(Deserialize)]
pub struct RecUpdate {
    pub name: String,
}

/// Update a recording
pub async fn update_recording(
    app: Extension<App>,
    Path((recording_id,)): Path<(RecordingId,)>,
    Json(update): Json<RecUpdate>,
) -> Result<Json<RecInfo>, AppError> {
    let rec = app.rename_recording(recording_id, update.name).await?;
    Ok(Json(rec.into()))
}

#[derive(Serialize, Deserialize)]
pub struct PlayRequest {
    id: RecordingId,
}

#[derive(Serialize, Deserialize)]
pub struct AppError {
    message: String,
}

impl From<color_eyre::eyre::ErrReport> for AppError {
    fn from(err: color_eyre::eyre::ErrReport) -> Self {
        AppError {
            message: err.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.message).into_response()
    }
}

#[axum_macros::debug_handler]
pub async fn play(
    app: Extension<App>,
    Json(request): Json<PlayRequest>,
) -> Result<Json<()>, AppError> {
    app.play_recording(request.id).await?;
    Ok(Json(()))
}

#[axum_macros::debug_handler]
pub async fn stop(app: Extension<App>, Json(()): Json<()>) -> Json<()> {
    app.stop_playing().await;
    Json(())
}

pub async fn play_status(app: Extension<App>) -> Json<Option<RecordingId>> {
    Json(app.playing_recording().await)
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum UpdateEvent {
    RecordBegin,
    RecordEnd { recording: RecInfo },
    RecordDelete { recording_id: RecordingId },
    RecordError { message: String },
    RecordUpdate { recording: RecInfo },
    PlayBegin { recording: RecordingId },
    PlayEnd,
}

impl UpdateEvent {
    pub fn from_state_change(change: StateChange) -> Option<UpdateEvent> {
        match change {
            StateChange::ListenBegin { .. } => None,
            StateChange::ListenEnd => None,
            StateChange::RecordBegin => Some(UpdateEvent::RecordBegin),
            StateChange::RecordEnd { recording } => Some(UpdateEvent::RecordEnd {
                recording: RecInfo::from(recording),
            }),
            StateChange::RecordUpdate { recording } => Some(UpdateEvent::RecordUpdate {
                recording: RecInfo::from(recording),
            }),
            StateChange::RecordError { message } => Some(UpdateEvent::RecordError { message }),
            StateChange::PlayBegin { recording } => Some(UpdateEvent::PlayBegin {
                recording: recording,
            }),
            StateChange::PlayEnd => Some(UpdateEvent::PlayEnd),
            StateChange::RecordDelete { recording_id } => {
                Some(UpdateEvent::RecordDelete { recording_id })
            }
        }
    }
}

pub async fn updates_sse(app: Extension<App>) -> impl IntoResponse {
    let mut events = app.subscribe();

    let source = async_stream::stream! {
        yield Event::default().comment("Welcome!");
        loop {
            match events.recv().await {
                Ok(change) => {
                    let update_event = UpdateEvent::from_state_change(change);
                    let sse_event = Event::default().json_data(update_event).expect("event type can be serialized");
                    yield sse_event;
                },
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                },
                Err(_) => break,
            }
        }
    };
    let stream = source.map(Result::<_, Infallible>::Ok);

    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    response.headers_mut().insert(
        "Cache-Control",
        HeaderValue::from_static("no-cache, no-store, no-transform, must-revalidate"),
    );
    response
}
