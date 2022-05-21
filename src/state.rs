use std::{sync::Arc, collections::HashMap};
use tokio::sync::Mutex;

use crate::midi::{Device, DeviceInfo};

pub type AppStateRef = Arc<Mutex<AppState>>;


#[derive(Default, Debug)]
pub struct AppState {
    pub devices: HashMap<Device, DeviceInfo>,
}

impl AppState {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn new_shared() -> AppStateRef {
        Arc::new(Mutex::new(AppState::new()))
    }
}
