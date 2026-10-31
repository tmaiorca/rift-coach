//! Frames on disk: `vision/<live_game_id>/<game time ms>.jpg`. Nothing else
//! is stored with them; the folder of a game is removed after its review.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::VisionError;

#[derive(Debug, Clone)]
pub struct FrameStore {
    dir: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct DiskUsage {
    pub games: u32,
    pub frames: u32,
    pub bytes: u64,
}

impl FrameStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn game_dir(&self, live_game_id: i64) -> PathBuf {
        self.dir.join(live_game_id.to_string())
    }

    pub fn write(&self, live_game_id: i64, t_ms: i64, jpeg: &[u8]) -> Result<(), VisionError> {
        let dir = self.game_dir(live_game_id);
        std::fs::create_dir_all(&dir)?;
        // Write then rename: a frame is either complete or absent.
        let tmp = dir.join(format!("{t_ms:08}.tmp"));
        std::fs::write(&tmp, jpeg)?;
        std::fs::rename(&tmp, dir.join(format!("{t_ms:08}.jpg")))?;
        Ok(())
    }

    /// Game times (ms) of a game's frames, ascending.
    pub fn list(&self, live_game_id: i64) -> Vec<i64> {
        let Ok(rd) = std::fs::read_dir(self.game_dir(live_game_id)) else { return vec![] };
        let mut out: Vec<i64> = rd
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                name.strip_suffix(".jpg")?.parse().ok()
            })
            .collect();
        out.sort_unstable();
        out
    }

    pub fn read(&self, live_game_id: i64, t_ms: i64) -> Result<Vec<u8>, VisionError> {
        Ok(std::fs::read(self.game_dir(live_game_id).join(format!("{t_ms:08}.jpg")))?)
    }

    /// Removes a game's frames. Returns whether there were any.
    pub fn delete(&self, live_game_id: i64) -> Result<bool, VisionError> {
        let dir = self.game_dir(live_game_id);
        if !dir.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(dir)?;
        Ok(true)
    }

    /// Live game ids that have a frames folder.
    pub fn games(&self) -> Vec<i64> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut ids: Vec<i64> = rd
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| e.file_name().into_string().ok()?.parse().ok())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Deletes the frames of every game `keep` rejects. Returns the games deleted.
    pub fn sweep(&self, keep: impl Fn(i64) -> bool) -> Vec<i64> {
        let mut deleted = Vec::new();
        for id in self.games() {
            if keep(id) {
                continue;
            }
            match self.delete(id) {
                Ok(_) => deleted.push(id),
                Err(e) => tracing::warn!(live_game_id = id, error = %e, "could not delete vision frames"),
            }
        }
        deleted
    }

    pub fn usage(&self) -> DiskUsage {
        let mut u = DiskUsage::default();
        for id in self.games() {
            u.games += 1;
            let Ok(rd) = std::fs::read_dir(self.game_dir(id)) else { continue };
            for e in rd.filter_map(Result::ok) {
                if let Ok(m) = e.metadata() {
                    u.frames += 1;
                    u.bytes += m.len();
                }
            }
        }
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_list_sweep() {
        let tmp = tempfile::tempdir().expect("tmp");
        let s = FrameStore::new(tmp.path().join("vision"));
        assert!(s.list(1).is_empty());
        s.write(1, 64_000, b"b").expect("w");
        s.write(1, 4_000, b"a").expect("w");
        s.write(2, 8_000, b"c").expect("w");
        assert_eq!(s.list(1), vec![4_000, 64_000]);
        assert_eq!(s.read(1, 4_000).expect("r"), b"a");
        assert_eq!(s.games(), vec![1, 2]);
        assert_eq!(s.usage(), DiskUsage { games: 2, frames: 3, bytes: 3 });
        assert_eq!(s.sweep(|id| id == 2), vec![1]);
        assert_eq!(s.games(), vec![2]);
        assert!(s.delete(2).expect("d"));
        assert!(!s.delete(2).expect("d"));
        assert_eq!(s.usage(), DiskUsage::default());
    }
}
