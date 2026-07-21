use super::catalog::{zip_cache_directory, CatalogError};
use super::client::{
    download_file_to, fetch_download_links_nxm, fetch_download_links_premium, fetch_mod_files,
    ClientError,
};
use super::domain::META_FILE;
use super::key::load_api_key;
use super::nxm::NxmLink;
use super::package::RemoteMod;
use crate::mods::{ensure_mod_folders, plugins_folder, ModError};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusMeta {
    pub source: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub game_domain: String,
    pub mod_name: String,
    pub version: String,
    pub author: String,
}

#[derive(Debug)]
pub enum InstallError {
    Catalog(CatalogError),
    Client(ClientError),
    MissingApiKey,
    NoDownloadableFile,
    NoDownloadLink,
    Zip(String),
    UnsafeArchivePath { entry: String },
    Io(io::Error),
    Mods(ModError),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "{error}"),
            Self::Client(error) => write!(formatter, "{error}"),
            Self::MissingApiKey => write!(
                formatter,
                "add your nexus mods api key in settings before installing"
            ),
            Self::NoDownloadableFile => {
                write!(formatter, "no primary downloadable file found for this mod")
            }
            Self::NoDownloadLink => write!(formatter, "nexus did not return a download link"),
            Self::Zip(detail) => write!(formatter, "zip error: {detail}"),
            Self::UnsafeArchivePath { entry } => {
                write!(formatter, "refusing unsafe archive path: {entry}")
            }
            Self::Io(error) => write!(formatter, "nexus install io error: {error}"),
            Self::Mods(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Client(error) => Some(error),
            Self::Io(error) => Some(error),
            Self::Mods(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<CatalogError> for InstallError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<ClientError> for InstallError {
    fn from(error: ClientError) -> Self {
        Self::Client(error)
    }
}

impl From<ModError> for InstallError {
    fn from(error: ModError) -> Self {
        Self::Mods(error)
    }
}

pub fn install_premium_mod(
    install_folder: &Path,
    remote: &RemoteMod,
) -> Result<PathBuf, InstallError> {
    let api_key = load_api_key()
        .map_err(|error| InstallError::Io(io::Error::other(error.to_string())))?
        .ok_or(InstallError::MissingApiKey)?;
    let files = fetch_mod_files(&api_key, &remote.domain_name, remote.mod_id)?;
    let file = pick_primary_file(&files.files).ok_or(InstallError::NoDownloadableFile)?;
    let links =
        fetch_download_links_premium(&api_key, &remote.domain_name, remote.mod_id, file.file_id)?;
    let download_url = links
        .first()
        .map(|link| link.uri.clone())
        .ok_or(InstallError::NoDownloadLink)?;
    let version = if file.version.trim().is_empty() {
        remote.version.clone()
    } else {
        file.version.clone()
    };
    download_and_install(
        install_folder,
        &api_key,
        &download_url,
        NexusMeta {
            source: "nexus".to_string(),
            mod_id: remote.mod_id,
            file_id: file.file_id,
            game_domain: remote.domain_name.clone(),
            mod_name: remote.name.clone(),
            version,
            author: remote.author.clone(),
        },
    )
}

pub fn install_from_nxm(install_folder: &Path, link: &NxmLink) -> Result<PathBuf, InstallError> {
    let api_key = load_api_key()
        .map_err(|error| InstallError::Io(io::Error::other(error.to_string())))?
        .ok_or(InstallError::MissingApiKey)?;
    let links = match (link.key.as_deref(), link.expires.as_deref()) {
        (Some(key), Some(expires)) => fetch_download_links_nxm(
            &api_key,
            &link.game_domain,
            link.mod_id,
            link.file_id,
            key,
            expires,
        )?,
        _ => fetch_download_links_premium(
            &api_key,
            &link.game_domain,
            link.mod_id,
            link.file_id,
        )?,
    };
    let download_url = links
        .first()
        .map(|entry| entry.uri.clone())
        .ok_or(InstallError::NoDownloadLink)?;

    let files = fetch_mod_files(&api_key, &link.game_domain, link.mod_id).ok();
    let (mod_name, version, author) = files
        .as_ref()
        .and_then(|list| {
            list.files
                .iter()
                .find(|file| file.file_id == link.file_id)
                .map(|file| {
                    (
                        if file.name.is_empty() {
                            format!("nexus-{}", link.mod_id)
                        } else {
                            file.name.clone()
                        },
                        file.version.clone(),
                        String::new(),
                    )
                })
        })
        .unwrap_or_else(|| {
            (
                format!("nexus-{}", link.mod_id),
                String::new(),
                String::new(),
            )
        });

    download_and_install(
        install_folder,
        &api_key,
        &download_url,
        NexusMeta {
            source: "nexus".to_string(),
            mod_id: link.mod_id,
            file_id: link.file_id,
            game_domain: link.game_domain.clone(),
            mod_name,
            version,
            author,
        },
    )
}

fn download_and_install(
    install_folder: &Path,
    api_key: &str,
    download_url: &str,
    meta: NexusMeta,
) -> Result<PathBuf, InstallError> {
    ensure_mod_folders(install_folder)?;
    let cache_dir = zip_cache_directory()?;
    fs::create_dir_all(&cache_dir)?;
    let archive_name = format!("nexus-{}-{}.zip", meta.mod_id, meta.file_id);
    let archive_path = cache_dir.join(archive_name);
    if !(archive_path.is_file() && archive_path.metadata()?.len() > 0) {
        download_file_to(api_key, download_url, &archive_path)?;
    }

    let entry_name = format!("nexus-{}", meta.mod_id);
    let destination = plugins_folder(install_folder).join(&entry_name);
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::create_dir_all(&destination)?;
    extract_archive_into(&archive_path, &destination)?;
    write_meta(&destination, &meta)?;
    write_manifest(&destination, &meta)?;
    Ok(destination)
}

fn pick_primary_file(files: &[super::package::ApiFile]) -> Option<&super::package::ApiFile> {
    files
        .iter()
        .find(|file| file.is_primary)
        .or_else(|| {
            files.iter().find(|file| {
                file.category_name.eq_ignore_ascii_case("MAIN")
                    || file.category_name.eq_ignore_ascii_case("main")
            })
        })
        .or_else(|| files.first())
}

fn write_meta(destination: &Path, meta: &NexusMeta) -> Result<(), InstallError> {
    let text = serde_json::to_string_pretty(meta)
        .map_err(|error| InstallError::Zip(error.to_string()))?;
    fs::write(destination.join(META_FILE), text)?;
    Ok(())
}

fn write_manifest(destination: &Path, meta: &NexusMeta) -> Result<(), InstallError> {
    let manifest_path = destination.join("manifest.json");
    if manifest_path.is_file() {
        return Ok(());
    }
    let payload = serde_json::json!({
        "name": meta.mod_name,
        "version_number": if meta.version.is_empty() { "0.0.0" } else { &meta.version },
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|error| InstallError::Zip(error.to_string()))?;
    let mut file = File::create(manifest_path)?;
    file.write_all(text.as_bytes())?;
    Ok(())
}

fn extract_archive_into(archive_path: &Path, destination: &Path) -> Result<(), InstallError> {
    let lower = archive_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower == "zip" || looks_like_zip(archive_path) {
        return extract_zip_into(archive_path, destination);
    }
    let file_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download.bin");
    fs::copy(archive_path, destination.join(file_name))?;
    Ok(())
}

fn looks_like_zip(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    bytes.len() >= 4 && bytes[0] == 0x50 && bytes[1] == 0x4B
}

fn extract_zip_into(archive_path: &Path, destination: &Path) -> Result<(), InstallError> {
    let file = File::open(archive_path)?;
    let mut archive =
        ZipArchive::new(file).map_err(|error| InstallError::Zip(error.to_string()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| InstallError::Zip(error.to_string()))?;
        let raw_name = entry.name().to_string();
        if raw_name.ends_with('/') {
            let relative = PathBuf::from(&raw_name);
            ensure_safe_relative(&relative, &raw_name)?;
            fs::create_dir_all(destination.join(relative))?;
            continue;
        }

        let relative = PathBuf::from(&raw_name);
        ensure_safe_relative(&relative, &raw_name)?;
        let output_path = destination.join(&relative);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&output_path)?;
        io::copy(&mut entry, &mut output)?;
    }

    Ok(())
}

fn ensure_safe_relative(relative: &Path, entry_name: &str) -> Result<(), InstallError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(InstallError::UnsafeArchivePath {
            entry: entry_name.to_string(),
        });
    }
    Ok(())
}

pub fn read_meta(folder: &Path) -> Option<NexusMeta> {
    let text = fs::read_to_string(folder.join(META_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}
