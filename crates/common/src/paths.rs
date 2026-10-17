use std::path::{Path, PathBuf};

/// Environment variable that relocates all app data (used by tests and dev runs).
pub const HOME_ENV: &str = "RIFT_COACH_HOME";

/// On-disk layout. Everything lives under one root so backup/export/delete
/// is a single directory operation.
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub db_file: PathBuf,
    pub logs_dir: PathBuf,
    pub ddragon_dir: PathBuf,
}

impl AppPaths {
    /// `%RIFT_COACH_HOME%` if set, otherwise `%LOCALAPPDATA%\RiftCoach`.
    pub fn resolve() -> std::io::Result<Self> {
        let root = match std::env::var_os(HOME_ENV) {
            Some(p) => PathBuf::from(p),
            None => dirs::data_local_dir()
                .ok_or_else(|| std::io::Error::other("cannot locate the local AppData folder"))?
                .join("RiftCoach"),
        };
        Self::at(root)
    }

    pub fn at(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        let paths = Self {
            db_file: root.join("rift-coach.db"),
            logs_dir: root.join("logs"),
            ddragon_dir: root.join("cache").join("ddragon"),
            root,
        };
        for dir in [&paths.root, &paths.logs_dir, &paths.ddragon_dir] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(paths)
    }
}
