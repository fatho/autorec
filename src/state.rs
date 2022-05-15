use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {

}

impl AppState {
    fn new() -> Self {
        Self {}
    }
}

pub fn init_state() -> Arc<Mutex<AppState>> {
    Arc::new(Mutex::new(AppState::new()))
}
