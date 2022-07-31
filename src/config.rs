use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub app: AppConfig,
    pub web: WebConfig,
}

#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    pub data_directory: PathBuf,
    pub midi_device: String,
}

#[derive(Serialize, Deserialize)]
pub struct WebConfig {
    pub port: u16,
    pub serve_frontend: Option<PathBuf>,
}
