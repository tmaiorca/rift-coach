//! Data Dragon static-data cache, versioned by patch.
//!
//! Layout: `<cache>/versions.json` and `<cache>/<version>/data/en_US/*.json`,
//! `<cache>/<version>/img/champion/*.png`. Data files for a version are
//! fetched once (tracked in the `ddragon_versions` table). Images are fetched
//! lazily on first use. Data Dragon is a static CDN and does not count
//! against Riot API rate limits.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rc_store::Db;
use serde::Deserialize;

pub const DEFAULT_BASE_URL: &str = "https://ddragon.leagueoflegends.com";
const LANG: &str = "en_US";
const DATA_FILES: [&str; 4] = ["champion.json", "item.json", "runesReforged.json", "summoner.json"];
const VERSIONS_TTL: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, thiserror::Error)]
pub enum DdragonError {
    #[error("Data Dragon request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Data Dragon returned HTTP {status} for {what}")]
    Status { status: u16, what: String },
    #[error("cache io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad Data Dragon json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("store error: {0}")]
    Store(#[from] rc_store::StoreError),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, DdragonError>;

/// `"16.20.1"` or `"16.20.712.3456"` → `"16.20"`.
pub fn patch_key(version: &str) -> String {
    let mut it = version.split('.');
    match (it.next(), it.next()) {
        (Some(a), Some(b)) => format!("{a}.{b}"),
        _ => version.to_owned(),
    }
}

fn is_release_version(v: &str) -> bool {
    !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// Picks the newest Data Dragon version for a game patch. `versions` is
/// newest-first, as Data Dragon publishes it. Falls back to the newest overall.
pub fn version_for_patch<'a>(versions: &'a [String], patch: &str) -> Option<&'a str> {
    versions.iter().find(|v| patch_key(v) == patch).or_else(|| versions.first()).map(String::as_str)
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ChampionInfo {
    /// Data Dragon id, e.g. `MonkeyKing`.
    pub id: String,
    /// Display name, e.g. `Wukong`.
    pub name: String,
    pub image_file: String,
}

#[derive(Debug, Default)]
pub struct ChampionIndex {
    by_key: HashMap<i32, ChampionInfo>,
}

impl ChampionIndex {
    pub fn get(&self, champion_id: i32) -> Option<&ChampionInfo> {
        self.by_key.get(&champion_id)
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (i32, &ChampionInfo)> {
        self.by_key.iter().map(|(k, v)| (*k, v))
    }

    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    fn parse(json: &[u8]) -> Result<Self> {
        #[derive(Deserialize)]
        struct File {
            data: HashMap<String, Champ>,
        }
        #[derive(Deserialize)]
        struct Champ {
            id: String,
            key: String,
            name: String,
            image: Image,
        }
        #[derive(Deserialize)]
        struct Image {
            full: String,
        }
        let file: File = serde_json::from_slice(json)?;
        let by_key = file
            .data
            .into_values()
            .filter_map(|c| {
                let key = c.key.parse().ok()?;
                Some((key, ChampionInfo { id: c.id, name: c.name, image_file: c.image.full }))
            })
            .collect();
        Ok(Self { by_key })
    }
}

#[derive(Debug)]
pub struct DdragonCache {
    http: reqwest::Client,
    base_url: String,
    dir: PathBuf,
    db: Db,
    versions: Mutex<Option<(Instant, Arc<Vec<String>>)>>,
    champions: Mutex<HashMap<String, Arc<ChampionIndex>>>,
    /// Serializes downloads so concurrent callers don't fetch the same files.
    download_lock: tokio::sync::Mutex<()>,
}

impl DdragonCache {
    pub fn new(dir: impl Into<PathBuf>, db: Db) -> Result<Self> {
        Self::with_base_url(dir, db, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(dir: impl Into<PathBuf>, db: Db, base_url: &str) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("RiftCoach/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_owned(),
            dir: dir.into(),
            db,
            versions: Mutex::new(None),
            champions: Mutex::new(HashMap::new()),
            download_lock: tokio::sync::Mutex::new(()),
        })
    }

    async fn fetch(&self, path: &str) -> Result<bytes::Bytes> {
        let url = format!("{}/{}", self.base_url, path);
        let resp = self.http.get(&url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(DdragonError::Status { status: status.as_u16(), what: path.to_owned() });
        }
        Ok(resp.bytes().await?)
    }

    /// Release versions, newest first. Cached in memory for a few hours and on
    /// disk as an offline fallback.
    pub async fn versions(&self) -> Result<Arc<Vec<String>>> {
        if let Some((at, v)) = self.versions.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_ref()
            && at.elapsed() < VERSIONS_TTL
        {
            return Ok(v.clone());
        }
        let disk = self.dir.join("versions.json");
        let raw = match self.fetch("api/versions.json").await {
            Ok(body) => {
                write_atomic(&disk, &body).await?;
                body.to_vec()
            }
            Err(e) => {
                tracing::warn!(error = %e, "Data Dragon versions fetch failed, using disk cache");
                tokio::fs::read(&disk).await.map_err(|_| e)?
            }
        };
        let all: Vec<String> = serde_json::from_slice(&raw)?;
        let list = Arc::new(all.into_iter().filter(|v| is_release_version(v)).collect::<Vec<_>>());
        if list.is_empty() {
            return Err(DdragonError::Invalid("Data Dragon returned no versions".into()));
        }
        *self.versions.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((Instant::now(), list.clone()));
        Ok(list)
    }

    /// Data Dragon version to use for a game patch like `"16.20"`.
    pub async fn version_for_patch(&self, patch: &str) -> Result<String> {
        let versions = self.versions().await?;
        version_for_patch(&versions, patch)
            .map(str::to_owned)
            .ok_or_else(|| DdragonError::Invalid("no Data Dragon versions".into()))
    }

    fn data_dir(&self, version: &str) -> PathBuf {
        self.dir.join(version).join("data").join(LANG)
    }

    /// Downloads the data files for `version` if not cached yet.
    pub async fn ensure_version(&self, version: &str) -> Result<()> {
        if !is_release_version(version) {
            return Err(DdragonError::Invalid(format!("invalid Data Dragon version '{version}'")));
        }
        let data_dir = self.data_dir(version);
        let cached = |data_dir: &Path| DATA_FILES.iter().all(|f| data_dir.join(f).exists());
        let v = version.to_owned();
        if self.db.call(move |c| rc_store::ddragon::is_complete(c, &v)).await? && cached(&data_dir) {
            return Ok(());
        }
        let _guard = self.download_lock.lock().await;
        if cached(&data_dir) {
            let v = version.to_owned();
            self.db.call(move |c| rc_store::ddragon::mark_complete(c, &v)).await?;
            return Ok(());
        }
        tokio::fs::create_dir_all(&data_dir).await?;
        for file in DATA_FILES {
            let body = self.fetch(&format!("cdn/{version}/data/{LANG}/{file}")).await?;
            serde_json::from_slice::<serde::de::IgnoredAny>(&body)?;
            write_atomic(&data_dir.join(file), &body).await?;
        }
        let v = version.to_owned();
        self.db.call(move |c| rc_store::ddragon::mark_complete(c, &v)).await?;
        tracing::info!(version, "Data Dragon version cached");
        Ok(())
    }

    /// Raw cached data file (e.g. `item.json`) for a version.
    pub async fn data_file(&self, version: &str, file: &str) -> Result<Vec<u8>> {
        if !DATA_FILES.contains(&file) {
            return Err(DdragonError::Invalid(format!("unknown data file '{file}'")));
        }
        self.ensure_version(version).await?;
        Ok(tokio::fs::read(self.data_dir(version).join(file)).await?)
    }

    pub async fn champions(&self, version: &str) -> Result<Arc<ChampionIndex>> {
        if let Some(idx) = self.champions.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(version) {
            return Ok(idx.clone());
        }
        let idx = Arc::new(ChampionIndex::parse(&self.data_file(version, "champion.json").await?)?);
        self.champions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(version.to_owned(), idx.clone());
        Ok(idx)
    }

    /// Local path of a champion's square icon, downloading it on first use.
    pub async fn champion_icon(&self, version: &str, champion_id: i32) -> Result<PathBuf> {
        let champs = self.champions(version).await?;
        let info = champs
            .get(champion_id)
            .ok_or_else(|| DdragonError::Invalid(format!("unknown champion id {champion_id} in {version}")))?;
        let file = &info.image_file;
        if file.contains(['/', '\\']) || file.contains("..") {
            return Err(DdragonError::Invalid(format!("suspicious image name '{file}'")));
        }
        let path = self.dir.join(version).join("img").join("champion").join(file);
        if !path.exists() {
            let body = self.fetch(&format!("cdn/{version}/img/champion/{file}")).await?;
            write_atomic(&path, &body).await?;
        }
        Ok(path)
    }
}

impl DdragonCache {
    /// Downloads `cdn_path` to `local` once, then serves the local copy.
    async fn cached(&self, cdn_path: &str, local: PathBuf) -> Result<PathBuf> {
        if !local.exists() {
            let body = self.fetch(cdn_path).await?;
            write_atomic(&local, &body).await?;
        }
        Ok(local)
    }

    pub async fn item_icon(&self, version: &str, item_id: i32) -> Result<PathBuf> {
        if item_id <= 0 || !is_release_version(version) {
            return Err(DdragonError::Invalid(format!("bad item {item_id} / version {version}")));
        }
        let local = self.dir.join(version).join("img").join("item").join(format!("{item_id}.png"));
        self.cached(&format!("cdn/{version}/img/item/{item_id}.png"), local).await
    }

    /// Icon for a summoner spell by its numeric key (e.g. 4 = Flash).
    pub async fn summoner_spell_icon(&self, version: &str, spell_key: i32) -> Result<PathBuf> {
        #[derive(Deserialize)]
        struct File {
            data: HashMap<String, Spell>,
        }
        #[derive(Deserialize)]
        struct Spell {
            key: String,
            image: Image,
        }
        #[derive(Deserialize)]
        struct Image {
            full: String,
        }
        let file: File = serde_json::from_slice(&self.data_file(version, "summoner.json").await?)?;
        let image = file
            .data
            .into_values()
            .find(|s| s.key == spell_key.to_string())
            .map(|s| s.image.full)
            .ok_or_else(|| DdragonError::Invalid(format!("unknown summoner spell {spell_key}")))?;
        if !safe_name(&image) {
            return Err(DdragonError::Invalid(format!("suspicious image name '{image}'")));
        }
        let local = self.dir.join(version).join("img").join("spell").join(&image);
        self.cached(&format!("cdn/{version}/img/spell/{image}"), local).await
    }

    /// Full splash art (`<id>_0.jpg`) for a champion's Data Dragon id
    /// (e.g. `Leblanc`). Splashes aren't versioned.
    pub async fn champion_splash(&self, ddragon_id: &str) -> Result<PathBuf> {
        if ddragon_id.is_empty() || !ddragon_id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(DdragonError::Invalid(format!("bad champion id '{ddragon_id}'")));
        }
        let file = format!("{ddragon_id}_0.jpg");
        let local = self.dir.join("splash").join(&file);
        self.cached(&format!("cdn/img/champion/splash/{file}"), local).await
    }

    /// Summoner's Rift minimap image.
    pub async fn minimap(&self, version: &str) -> Result<PathBuf> {
        if !is_release_version(version) {
            return Err(DdragonError::Invalid(format!("invalid version '{version}'")));
        }
        let local = self.dir.join(version).join("img").join("map").join("map11.png");
        self.cached(&format!("cdn/{version}/img/map/map11.png"), local).await
    }
}

fn safe_name(file: &str) -> bool {
    !file.is_empty() && !file.contains(['/', '\\']) && !file.contains("..")
}

/// Write to a temp file then rename, so a crash never leaves a half file.
/// Each write gets its own temp name: concurrent writers of the same file
/// (e.g. several icon requests refreshing `versions.json`) must not share one.
async fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}-{n}.part", std::process::id()));
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_mapping() {
        assert_eq!(patch_key("16.20.1"), "16.20");
        assert_eq!(patch_key("16.20.712.3456"), "16.20");
        let versions: Vec<String> = ["16.21.1", "16.20.2", "16.20.1", "16.19.1"].map(String::from).to_vec();
        assert_eq!(version_for_patch(&versions, "16.20"), Some("16.20.2"));
        assert_eq!(version_for_patch(&versions, "15.1"), Some("16.21.1"), "unknown patch falls back to newest");
        assert!(is_release_version("16.20.1"));
        assert!(!is_release_version("lolpatch_3.7"));
        assert!(!is_release_version("../etc"));
    }
}
