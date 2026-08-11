use super::client::{fetch_mod_list_pages, ClientError};
use super::domain::{CACHE_ENV, CACHE_MAX_AGE_SECS, CATALOG_CACHE_VERSION, GAME_DOMAIN};
use super::key::load_api_key;
use super::package::{ApiMod, RemoteMod};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogCacheFile {
    #[serde(default)]
    schema_version: u32,
    fetched_at_unix: u64,
    mods: Vec<ApiMod>,
}

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub mods: Vec<RemoteMod>,
    pub fetched_at: SystemTime,
    pub from_cache: bool,
    pub network_warning: Option<String>,
}

#[derive(Debug)]
pub enum CatalogError {
    CacheDirectoryUnavailable,
    MissingApiKey,
    Client(ClientError),
    Decode(String),
    Io(io::Error),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CacheDirectoryUnavailable => {
                write!(
                    formatter,
                    "could not resolve a cache directory for nexus mods"
                )
            }
            Self::MissingApiKey => write!(
                formatter,
                "add your nexus mods api key in settings before browsing"
            ),
            Self::Client(error) => write!(formatter, "{error}"),
            Self::Decode(detail) => write!(formatter, "could not parse nexus cache: {detail}"),
            Self::Io(error) => write!(formatter, "nexus cache io error: {error}"),
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for CatalogError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ClientError> for CatalogError {
    fn from(error: ClientError) -> Self {
        Self::Client(error)
    }
}

pub fn load_catalog(force_refresh: bool) -> Result<CatalogSnapshot, CatalogError> {
    let cache_dir = cache_directory()?;
    fs::create_dir_all(&cache_dir)?;
    let cache_path = catalog_cache_path(&cache_dir);

    if !force_refresh {
        if let Some(snapshot) = read_fresh_cache(&cache_path)? {
            return Ok(snapshot);
        }
    }

    match fetch_and_store(&cache_path) {
        Ok(snapshot) => Ok(snapshot),
        Err(network_error) => {
            if matches!(
                &network_error,
                CatalogError::Client(ClientError::RateLimited)
            ) {
                if let Some(stale) = read_any_cache(&cache_path)? {
                    return Ok(CatalogSnapshot {
                        mods: stale.mods,
                        fetched_at: stale.fetched_at,
                        from_cache: true,
                        network_warning: Some(
                            "rate limited by nexus mods — showing cached list; try again later"
                                .to_string(),
                        ),
                    });
                }
                return Err(network_error);
            }
            if let Some(stale) = read_any_cache(&cache_path)? {
                Ok(CatalogSnapshot {
                    mods: stale.mods,
                    fetched_at: stale.fetched_at,
                    from_cache: true,
                    network_warning: Some(network_error.to_string()),
                })
            } else {
                Err(network_error)
            }
        }
    }
}

pub fn cache_age_label(fetched_at: SystemTime) -> String {
    let age = SystemTime::now()
        .duration_since(fetched_at)
        .unwrap_or_default();
    if age < Duration::from_secs(60) {
        "updated just now".to_string()
    } else if age < Duration::from_secs(3_600) {
        format!("updated {}m ago", age.as_secs() / 60)
    } else {
        format!("updated {}h ago", age.as_secs() / 3_600)
    }
}

fn fetch_and_store(cache_path: &Path) -> Result<CatalogSnapshot, CatalogError> {
    let api_key = load_api_key()
        .map_err(|error| CatalogError::Io(io::Error::other(error.to_string())))?
        .ok_or(CatalogError::MissingApiKey)?;
    let api_mods = fetch_mod_list_pages(&api_key, GAME_DOMAIN)?;
    let fetched_at = SystemTime::now();
    let fetched_at_unix = fetched_at
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let cache = CatalogCacheFile {
        schema_version: CATALOG_CACHE_VERSION,
        fetched_at_unix,
        mods: api_mods,
    };
    write_cache(cache_path, &cache)?;
    Ok(CatalogSnapshot {
        mods: flatten_mods(cache.mods),
        fetched_at,
        from_cache: false,
        network_warning: None,
    })
}

fn read_fresh_cache(cache_path: &Path) -> Result<Option<CatalogSnapshot>, CatalogError> {
    let Some(snapshot) = read_any_cache(cache_path)? else {
        return Ok(None);
    };
    let age = SystemTime::now()
        .duration_since(snapshot.fetched_at)
        .unwrap_or(Duration::MAX);
    if age <= Duration::from_secs(CACHE_MAX_AGE_SECS) {
        Ok(Some(snapshot))
    } else {
        Ok(None)
    }
}

fn read_any_cache(cache_path: &Path) -> Result<Option<CatalogSnapshot>, CatalogError> {
    if !cache_path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(cache_path)?;
    let cache: CatalogCacheFile =
        serde_json::from_str(&text).map_err(|error| CatalogError::Decode(error.to_string()))?;
    if cache.schema_version != CATALOG_CACHE_VERSION {
        return Ok(None);
    }
    let fetched_at = UNIX_EPOCH + Duration::from_secs(cache.fetched_at_unix);
    Ok(Some(CatalogSnapshot {
        mods: flatten_mods(cache.mods),
        fetched_at,
        from_cache: true,
        network_warning: None,
    }))
}

fn write_cache(cache_path: &Path, cache: &CatalogCacheFile) -> Result<(), CatalogError> {
    let text =
        serde_json::to_string(cache).map_err(|error| CatalogError::Decode(error.to_string()))?;
    let temporary = cache_path.with_extension("json.partial");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temporary, cache_path)?;
    Ok(())
}

fn flatten_mods(api_mods: Vec<ApiMod>) -> Vec<RemoteMod> {
    let mut mods: Vec<RemoteMod> = api_mods.into_iter().filter_map(RemoteMod::from_api).collect();
    mods.sort_by(|left, right| {
        right
            .endorsement_count
            .cmp(&left.endorsement_count)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    mods
}

fn catalog_cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("catalog.json")
}

pub fn cache_directory() -> Result<PathBuf, CatalogError> {
    if let Some(explicit) = std::env::var_os(CACHE_ENV) {
        return Ok(PathBuf::from(explicit));
    }
    Ok(crate::platform::cache_directory()
        .map_err(|_| CatalogError::CacheDirectoryUnavailable)?
        .join("nexus"))
}

pub fn zip_cache_directory() -> Result<PathBuf, CatalogError> {
    Ok(cache_directory()?.join("zips"))
}
