use super::community::{CACHE_ENV, CACHE_MAX_AGE_SECS, PACKAGE_LIST_URL};
use super::package::{ApiPackage, RemotePackage};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CatalogCacheFile {
    fetched_at_unix: u64,
    packages: Vec<ApiPackage>,
}

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub packages: Vec<RemotePackage>,
    pub fetched_at: SystemTime,
    pub from_cache: bool,
    pub network_warning: Option<String>,
}

#[derive(Debug)]
pub enum CatalogError {
    CacheDirectoryUnavailable,
    Network(String),
    Decode(String),
    Io(io::Error),
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CacheDirectoryUnavailable => {
                write!(
                    formatter,
                    "could not resolve a cache directory for thunderstore packages"
                )
            }
            Self::Network(detail) => write!(
                formatter,
                "could not reach thunderstore (check your network): {detail}"
            ),
            Self::Decode(detail) => {
                write!(formatter, "could not parse thunderstore response: {detail}")
            }
            Self::Io(error) => write!(formatter, "thunderstore cache io error: {error}"),
        }
    }
}

impl std::error::Error for CatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
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
            if let Some(stale) = read_any_cache(&cache_path)? {
                Ok(CatalogSnapshot {
                    packages: stale.packages,
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
    let body = download_package_list()?;
    let api_packages: Vec<ApiPackage> =
        serde_json::from_str(&body).map_err(|error| CatalogError::Decode(error.to_string()))?;
    let fetched_at = SystemTime::now();
    let fetched_at_unix = fetched_at
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let cache = CatalogCacheFile {
        fetched_at_unix,
        packages: api_packages,
    };
    write_cache(cache_path, &cache)?;

    Ok(CatalogSnapshot {
        packages: flatten_packages(cache.packages),
        fetched_at,
        from_cache: false,
        network_warning: None,
    })
}

fn download_package_list() -> Result<String, CatalogError> {
    let output = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "60",
        ])
        .arg(PACKAGE_LIST_URL)
        .output()
        .map_err(|error| CatalogError::Network(format!("failed to spawn curl: {error}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if stderr.trim().is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr.trim().to_string()
        };
        return Err(CatalogError::Network(detail));
    }

    String::from_utf8(output.stdout)
        .map_err(|error| CatalogError::Decode(format!("response was not utf-8: {error}")))
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
    let fetched_at = UNIX_EPOCH + Duration::from_secs(cache.fetched_at_unix);
    Ok(Some(CatalogSnapshot {
        packages: flatten_packages(cache.packages),
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

fn flatten_packages(api_packages: Vec<ApiPackage>) -> Vec<RemotePackage> {
    let mut packages: Vec<RemotePackage> = api_packages
        .into_iter()
        .filter_map(RemotePackage::from_api)
        .filter(|package| !package.is_deprecated)
        .collect();
    packages.sort_by(|left, right| {
        right
            .downloads
            .cmp(&left.downloads)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    packages
}

fn catalog_cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("catalog.json")
}

pub fn cache_directory() -> Result<PathBuf, CatalogError> {
    if let Some(explicit) = std::env::var_os(CACHE_ENV) {
        return Ok(PathBuf::from(explicit));
    }
    let home = std::env::var_os("HOME").ok_or(CatalogError::CacheDirectoryUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".cache")
        .join("silksong-mod-manager")
        .join("thunderstore"))
}

pub fn zip_cache_directory() -> Result<PathBuf, CatalogError> {
    Ok(cache_directory()?.join("zips"))
}
