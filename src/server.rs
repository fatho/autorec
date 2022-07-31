use std::{convert::Infallible, sync::Arc};

use axum::{
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
    store::{RecordingEntry, RecordingId},
};

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

#[derive(Serialize)]
pub struct RecInfo {
    pub id: RecordingId,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

impl RecInfo {
    pub fn new(entry: RecordingEntry) -> Self {
        RecInfo {
            id: entry.id,
            name: entry.name.clone(),
            created_at: entry.created_at,
        }
    }
}

/// Return list songs
pub async fn songs(app: Extension<Arc<App>>) -> Json<Vec<RecInfo>> {
    let songs = app.query_songs().await.map_or_else(
        |err| {
            error!("Failed to list songs: {}", err);
            vec![]
        },
        |songs| songs.into_iter().map(RecInfo::new).collect(),
    );

    Json(songs)
}

#[derive(Serialize, Deserialize)]
pub struct PlayRequest {
    id: RecordingId,
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
pub async fn play(
    app: Extension<Arc<App>>,
    Json(request): Json<PlayRequest>,
) -> Result<Json<()>, AppError> {
    app.play_recording(request.id).await;
    Ok(Json(()))
}

#[axum_macros::debug_handler]
pub async fn stop(app: Extension<Arc<App>>, Json(()): Json<()>) -> Json<()> {
    app.stop_playing().await;
    Json(())
}

pub async fn play_status(app: Extension<Arc<App>>) -> Json<Option<RecordingId>> {
    Json(app.playing_recording())
}

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum UpdateEvent {
    RecordBegin,
    RecordEnd { recording: RecInfo },
    RecordError { message: String },
    PlayBegin { recording: RecordingId },
    PlayError { message: String },
    PlayEnd,
}

impl UpdateEvent {
    pub fn from_state_change(change: StateChange) -> Option<UpdateEvent> {
        match change {
            StateChange::ListenBegin { .. } => None,
            StateChange::ListenEnd => None,
            StateChange::RecordBegin => Some(UpdateEvent::RecordBegin),
            StateChange::RecordEnd { recording } => Some(UpdateEvent::RecordEnd {
                recording: RecInfo::new(recording),
            }),
            StateChange::RecordError { message } => Some(UpdateEvent::RecordError { message }),
            StateChange::PlayBegin { recording } => Some(UpdateEvent::PlayBegin {
                recording: recording,
            }),
            StateChange::PlayEnd => Some(UpdateEvent::PlayEnd),
            StateChange::PlayError { message } => Some(UpdateEvent::PlayError { message }),
        }
    }
}

pub async fn updates_sse(app: Extension<Arc<App>>) -> impl IntoResponse {
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
