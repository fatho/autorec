use std::borrow::Cow;

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

macro_rules! template_source {
    ($name:expr) => {
        {
            #[cfg(debug_assertions)]
            fn get_source() -> std::io::Result<Cow<'static, str>> {
                std::fs::read_to_string(concat!("templates/", $name)).map(Cow::Owned)
            }

            #[cfg(not(debug_assertions))]
            fn get_source() -> std::io::Result<Cow<'static, str>> {
                Ok(Cow::Borrowed(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/", $name))))
            }

            get_source()
        }
    };
}

pub async fn startpage(state_ref: Extension<AppStateRef>) -> Result<axum::response::Html<String>, String> {
    let state = state_ref.lock().unwrap();
    let mut songs = state.query_songs().unwrap_or_else(|err| {
        error!("Failed to list songs: {}", err);
        vec![]
    });
    songs.sort_by(|a, b| b.cmp(a));

    let source = template_source!("main.html.liquid").map_err(|err| err.to_string())?;

    let template = liquid::ParserBuilder::with_stdlib()
        .build().map_err(|err| err.to_string())?
        .parse(&source).map_err(|err| err.to_string())?;

    let globals = liquid::object!({
        "songs": songs
    });

    let output = template.render(&globals).unwrap();

    Ok(axum::response::Html(output))
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

pub async fn stop(state_ref: Extension<AppStateRef>, Json(request): Json<()>) -> Result<Json<String>, AppError> {
    let mut state = state_ref.lock().unwrap();

    match state.stop_song() {
        Ok(()) => Ok(Json("ok".to_owned())),
        Err(err) => Err(AppError { message: err.to_string() }),
    }
}
