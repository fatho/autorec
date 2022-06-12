use axum::{Extension, Json};
use serde::{Serialize, Deserialize};

use crate::{state::AppStateRef};


#[derive(Serialize, Deserialize)]
pub struct DeviceObject {
    id: String,
    description: String,
}

/// Return list of devices
pub async fn devices(state_ref: Extension<AppStateRef>) -> Json<Vec<DeviceObject>> {
    let state = state_ref.lock().unwrap();

    let result = state.devices.iter().map(|(device, info)| DeviceObject {
        id: device.id(),
        description: format!("{} ({})", info.client_name, info.port_name),
    }).collect();

    Json(result)
}

/// Return debug-printed state
pub async fn debug(state_ref: Extension<AppStateRef>) -> String {
    let state = state_ref.lock().unwrap();

    format!("{:#?}", state)
}
