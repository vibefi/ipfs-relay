pub mod config;
pub mod error;
pub mod ipfs;
pub mod middleware;
pub mod models;
pub mod pinning;
pub mod routes;
pub mod validation;

pub use routes::{api_router, meta_router};

use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<config::AppConfig>,
    pub ipfs: Arc<ipfs::KuboClient>,
    pub pinning: Arc<pinning::PinningService>,
}
