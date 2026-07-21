use super::catalog::{zip_cache_directory, CatalogError};
use super::package::{is_modloader_pack, parse_dependency, version_is_newer, RemotePackage};
use crate::mods::{ensure_mod_folders, plugins_folder, InstalledMod, ModError};
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use zip::ZipArchive;

#[derive(Debug, Clone)]
pub struct MissingDependency {
    pub full_name: String,
    pub required_version: String,
    pub package: Option<RemotePackage>,
}

#[derive(Debug, Clone)]
pub struct InstallRequest {
    pub package: RemotePackage,
    pub include_dependencies: bool,
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub installed: Vec<String>,
    pub skipped_modloader_deps: Vec<String>,
}

#[derive(Debug)]
pub enum InstallError {
    Catalog(CatalogError),
    Network(String),
    Zip(String),
    UnsafeArchivePath { entry: String },
    MissingDependency { full_name: String },
    Io(io::Error),
    Mods(ModError),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "{error}"),
            Self::Network(detail) => {
                write!(formatter, "download failed (check your network): {detail}")
            }
            Self::Zip(detail) => write!(formatter, "zip error: {detail}"),
            Self::UnsafeArchivePath { entry } => {
                write!(formatter, "refusing unsafe archive path: {entry}")
            }
            Self::MissingDependency { full_name } => {
                write!(
                    formatter,
                    "dependency '{full_name}' is not in the thunderstore catalog"
                )
            }
            Self::Io(error) => write!(formatter, "install io error: {error}"),
            Self::Mods(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
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

impl From<ModError> for InstallError {
    fn from(error: ModError) -> Self {
        Self::Mods(error)
    }
}

pub fn find_update_version(
    installed: &InstalledMod,
    packages: &[RemotePackage],
) -> Option<String> {
    let remote = match_installed_package(installed, packages)?;
    let installed_version = installed.version.as_ref()?;
    if version_is_newer(&remote.version, installed_version) {
        Some(remote.version.clone())
    } else {
        None
    }
}

pub fn match_installed_package<'a>(
    installed: &InstalledMod,
    packages: &'a [RemotePackage],
) -> Option<&'a RemotePackage> {
    packages
        .iter()
        .find(|package| package.full_name == installed.entry_name)
        .or_else(|| {
            packages.iter().find(|package| {
                package.name.eq_ignore_ascii_case(&installed.display_name)
                    || package.name.eq_ignore_ascii_case(&installed.entry_name)
            })
        })
}

pub fn collect_missing_dependencies(
    package: &RemotePackage,
    installed: &[InstalledMod],
    catalog: &[RemotePackage],
) -> Result<(Vec<MissingDependency>, Vec<String>), InstallError> {
    let mut missing = Vec::new();
    let mut skipped_modloader = Vec::new();
    let mut visited = Vec::new();
    collect_missing_recursive(
        package,
        installed,
        catalog,
        &mut missing,
        &mut skipped_modloader,
        &mut visited,
    )?;
    Ok((missing, skipped_modloader))
}

fn collect_missing_recursive(
    package: &RemotePackage,
    installed: &[InstalledMod],
    catalog: &[RemotePackage],
    missing: &mut Vec<MissingDependency>,
    skipped_modloader: &mut Vec<String>,
    visited: &mut Vec<String>,
) -> Result<(), InstallError> {
    for dependency in &package.dependencies {
        let Some((full_name, required_version)) = parse_dependency(dependency) else {
            continue;
        };
        if visited.iter().any(|name| name == &full_name) {
            continue;
        }
        visited.push(full_name.clone());

        if is_modloader_pack(&full_name) {
            skipped_modloader.push(full_name);
            continue;
        }

        if dependency_satisfied(&full_name, &required_version, installed) {
            continue;
        }

        let remote = catalog
            .iter()
            .find(|entry| entry.full_name == full_name)
            .cloned();

        if let Some(remote_package) = remote.clone() {
            collect_missing_recursive(
                &remote_package,
                installed,
                catalog,
                missing,
                skipped_modloader,
                visited,
            )?;
        }

        if !missing.iter().any(|entry| entry.full_name == full_name) {
            missing.push(MissingDependency {
                full_name,
                required_version,
                package: remote,
            });
        }
    }
    Ok(())
}

fn dependency_satisfied(
    full_name: &str,
    required_version: &str,
    installed: &[InstalledMod],
) -> bool {
    installed.iter().any(|entry| {
        let name_only = full_name.rsplit_once('-').map(|(_, name)| name);
        let matches_identity = entry.entry_name == full_name
            || name_only == Some(entry.display_name.as_str())
            || name_only == Some(entry.entry_name.as_str());
        if !matches_identity {
            return false;
        }
        match &entry.version {
            Some(version) => {
                version == required_version || !version_is_newer(required_version, version)
            }
            None => true,
        }
    })
}

pub fn install_package_tree(
    install_folder: &Path,
    request: &InstallRequest,
    catalog: &[RemotePackage],
    installed: &[InstalledMod],
) -> Result<InstallReport, InstallError> {
    ensure_mod_folders(install_folder)?;

    let mut queue = Vec::new();
    let (missing, skipped_modloader_deps) = if request.include_dependencies {
        collect_missing_dependencies(&request.package, installed, catalog)?
    } else {
        let (_, skipped) = collect_missing_dependencies(&request.package, installed, catalog)?;
        (Vec::new(), skipped)
    };

    if request.include_dependencies {
        for dependency in missing {
            let Some(package) = dependency.package else {
                return Err(InstallError::MissingDependency {
                    full_name: dependency.full_name,
                });
            };
            queue.push(package);
        }
    }

    queue.push(request.package.clone());

    let mut installed_names = Vec::new();
    for package in queue {
        install_single_package(install_folder, &package)?;
        installed_names.push(package.full_name);
    }

    Ok(InstallReport {
        installed: installed_names,
        skipped_modloader_deps,
    })
}

pub fn install_from_download(
    install_folder: &Path,
    full_name: &str,
    version: &str,
    download_url: &str,
) -> Result<PathBuf, InstallError> {
    ensure_mod_folders(install_folder)?;
    let archive = download_zip(full_name, version, download_url)?;
    let destination = plugins_folder(install_folder).join(full_name);
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::create_dir_all(&destination)?;
    extract_zip_into(&archive, &destination)?;
    Ok(destination)
}

fn install_single_package(
    install_folder: &Path,
    package: &RemotePackage,
) -> Result<PathBuf, InstallError> {
    install_from_download(
        install_folder,
        &package.full_name,
        &package.version,
        &package.download_url,
    )
}

fn download_zip(
    full_name: &str,
    version: &str,
    download_url: &str,
) -> Result<PathBuf, InstallError> {
    let cache_dir = zip_cache_directory()?;
    fs::create_dir_all(&cache_dir)?;
    let archive_path = cache_dir.join(format!("{full_name}-{version}.zip"));
    if archive_path.is_file() && archive_path.metadata()?.len() > 0 {
        return Ok(archive_path);
    }

    let temporary = archive_path.with_extension("zip.partial");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--max-time",
            "120",
            "--output",
        ])
        .arg(&temporary)
        .arg(download_url)
        .status()
        .map_err(|error| InstallError::Network(format!("failed to spawn curl: {error}")))?;

    if !status.success() {
        let _ = fs::remove_file(&temporary);
        return Err(InstallError::Network(format!("curl exited with {status}")));
    }

    fs::rename(&temporary, &archive_path)?;
    Ok(archive_path)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-ts-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    fn write_sample_zip(path: &Path) {
        let file = File::create(path).expect("zip file");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        zip.start_file("manifest.json", options).expect("manifest");
        zip.write_all(
            br#"{ "name": "FsmUtil", "version_number": "0.3.17", "dependencies": [] }"#,
        )
        .expect("write manifest");
        zip.start_file("FsmUtil.dll", options).expect("dll");
        zip.write_all(b"dll-bytes").expect("write dll");
        zip.finish().expect("finish");
    }

    #[test]
    fn extracts_package_zip_into_plugins_folder() {
        let root = scratch("extract");
        let archive = root.join("pkg.zip");
        write_sample_zip(&archive);
        let destination = root.join("plugins").join("silksong_modding-FsmUtil");
        fs::create_dir_all(&destination).expect("dest");
        extract_zip_into(&archive, &destination).expect("extract");
        assert!(destination.join("manifest.json").is_file());
        assert!(destination.join("FsmUtil.dll").is_file());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn finds_update_when_remote_is_newer() {
        let installed = InstalledMod {
            entry_name: "owner-Sample".into(),
            display_name: "Sample".into(),
            version: Some("1.0.0".into()),
            enabled: true,
            path: PathBuf::from("/tmp/owner-Sample"),
            kind: crate::mods::ModKind::Folder,
            source: crate::mods::ModSource::Thunderstore,
        };
        let remote = RemotePackage {
            name: "Sample".into(),
            full_name: "owner-Sample".into(),
            owner: "owner".into(),
            description: String::new(),
            version: "1.1.0".into(),
            version_full_name: "owner-Sample-1.1.0".into(),
            downloads: 1,
            download_url: "https://example.invalid".into(),
            icon_url: String::new(),
            dependencies: Vec::new(),
            date_updated: String::new(),
            is_deprecated: false,
        };
        assert_eq!(
            find_update_version(&installed, &[remote]).as_deref(),
            Some("1.1.0")
        );
    }
}
