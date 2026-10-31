use std::sync::Arc;

use rc_common::{AppPaths, Secret};
use rc_ddragon::DdragonCache;
use rc_riot::{RateLimiter, RiotClient};
use rc_store::Db;

use crate::engine::{Engine, EngineConfig};

#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    #[error(transparent)]
    Store(#[from] rc_store::StoreError),
    #[error(transparent)]
    Riot(#[from] rc_riot::RiotError),
    #[error(transparent)]
    Ddragon(#[from] rc_ddragon::DdragonError),
}

/// Wires DB, Riot client, Data Dragon cache, and engine the same way for the
/// app and the CLI. Workers are not started; call [`Engine::start`].
pub fn build_engine(
    paths: &AppPaths,
    key: Option<Secret>,
    config: EngineConfig,
) -> Result<Arc<Engine>, BootstrapError> {
    let db = Db::open(&paths.db_file)?;
    let riot = Arc::new(RiotClient::new(key, Arc::new(RateLimiter::personal_key()))?);
    let ddragon = Arc::new(DdragonCache::new(&paths.ddragon_dir, db.clone())?);
    let engine = Engine::new(db, riot, ddragon, config);
    engine.set_frame_store(rc_vision::FrameStore::new(&paths.vision_dir));
    Ok(engine)
}
