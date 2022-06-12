use axum::{Extension, Json};
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

pub async fn startpage(state_ref: Extension<AppStateRef>) -> axum::response::Html<String> {
    use build_html::*;

    let state = state_ref.lock().unwrap();
    let songs = state.query_songs().unwrap_or_else(|err| {
        error!("Failed to list songs: {}", err);
        vec![]
    });

    let mut songs_html = Container::new(ContainerType::OrderedList);

    let songs_js = serde_json::to_string(&songs).unwrap();

    for (index, song) in songs.iter().enumerate() {
        songs_html.add_link_attr("#", song, [("onclick", format!("play({index})").as_str())]);
    }

    let page = HtmlPage::new()
        .with_script_literal(format!("var songs = {songs_js};"))
        .with_script_literal(r"
        async function play(index) {
            await fetch(
                '/play',
                {
                    'method': 'POST',
                    headers: {
                        'Accept': 'application/json',
                        'Content-Type': 'application/json'
                    },
                    body: JSON.stringify({'name': songs[index]})
                }
            );
        }")
        .with_title("AutoRec")
        .with_header(1, "AutoRec")
        .with_container(
            Container::new(ContainerType::Article)
                .with_attributes([("id", "article1")])
                .with_header(2, "Recorded Songs")
                .with_container(songs_html),
        );

    axum::response::Html(page.to_html_string())
}


#[derive(Serialize, Deserialize)]
pub struct PlayRequest {
    name: String,
}

pub async fn play(state_ref: Extension<AppStateRef>, Json(request): Json<PlayRequest>) -> Result<Json<String>, Json<String>> {
    let mut state = state_ref.lock().unwrap();

    match state.play_song(request.name) {
        Ok(()) => Ok(Json("ok".to_owned())),
        Err(err) => Err(Json(err.to_string())),
    }
}
