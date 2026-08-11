use super::launch::{LaunchOptionsOutcome, LaunchScriptAction};
use super::pack::{
    pack_cache_file_name, pack_download_url, PackError, BEPINEX_FOLDER, MANAGER_BACKUP_FOLDER,
    PACK_ROOT_PREFIX, RECOMMENDED_PACK_VERSION, RUN_SCRIPT, SMM_LAUNCH_SCRIPT,
};
use super::status::manager_pack_version_path;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use zip::ZipArchive;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallEvent {
    WouldCreateDirectory { path: PathBuf },
    WouldWriteFile { path: PathBuf, bytes: u64 },
    WouldBackupFile { from: PathBuf, to: PathBuf },
    WouldSetExecutable { path: PathBuf },
    WouldWritePackVersion { path: PathBuf, version: String },
    CreatedDirectory { path: PathBuf },
    WroteFile { path: PathBuf, bytes: u64 },
    BackedUpFile { from: PathBuf, to: PathBuf },
    SetExecutable { path: PathBuf },
    WrotePackVersion { path: PathBuf, version: String },
}

#[derive(Debug, Clone)]
pub struct PlannedFile {
    pub relative_path: PathBuf,
    pub destination: PathBuf,
    pub destination_exists: bool,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub archive_path: PathBuf,
    pub install_folder: PathBuf,
    pub backup_root: PathBuf,
    pub files: Vec<PlannedFile>,
    pub events: Vec<InstallEvent>,
}

#[derive(Debug, Clone)]
pub struct InstallSummary {
    pub dry_run: bool,
    pub events: Vec<InstallEvent>,
    pub launch: LaunchOptionsOutcome,
    pub backup_root: PathBuf,
}

impl InstallSummary {
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        if let Some(alert) = self.launch.alert_message() {
            lines.push(alert);
            lines.push(String::new());
        }
        if self.dry_run {
            lines.push("dry run — no files were changed".to_string());
        } else {
            lines.push("bepinex install finished".to_string());
        }
        lines.push(format!("backup folder: {}", self.backup_root.display()));
        for event in &self.events {
            lines.push(format!("- {}", describe_event(event)));
        }
        lines.push(format!("- {}", describe_launch(&self.launch)));
        lines.join("\n")
    }
}

#[derive(Debug)]
pub enum InstallError {
    Pack(PackError),
    Zip(String),
    UnsafeArchivePath { entry: String },
    MissingPackRoot,
    Io(io::Error),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pack(error) => write!(formatter, "{error}"),
            Self::Zip(detail) => write!(formatter, "zip error: {detail}"),
            Self::UnsafeArchivePath { entry } => {
                write!(formatter, "refusing unsafe archive path: {entry}")
            }
            Self::MissingPackRoot => write!(
                formatter,
                "archive does not contain a {PACK_ROOT_PREFIX} root folder"
            ),
            Self::Io(error) => write!(formatter, "install io error: {error}"),
        }
    }
}

impl std::error::Error for InstallError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Pack(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<PackError> for InstallError {
    fn from(error: PackError) -> Self {
        Self::Pack(error)
    }
}

pub fn download_recommended_pack() -> Result<PathBuf, InstallError> {
    let cache_dir = cache_directory()?;
    fs::create_dir_all(&cache_dir)?;
    let archive_path = cache_dir.join(pack_cache_file_name());
    if archive_path.is_file() && archive_path.metadata()?.len() > 0 {
        return Ok(archive_path);
    }

    let url = pack_download_url();
    let temporary_path = archive_path.with_extension("zip.partial");
    let status = std::process::Command::new("curl")
        .args(["--fail", "--location", "--silent", "--show-error", "--output"])
        .arg(&temporary_path)
        .arg(&url)
        .status()
        .map_err(|error| PackError::DownloadFailed {
            detail: format!("failed to spawn curl: {error}"),
        })?;

    if !status.success() {
        let _ = fs::remove_file(&temporary_path);
        return Err(PackError::DownloadFailed {
            detail: format!("curl exited with {status}"),
        }
        .into());
    }

    fs::rename(&temporary_path, &archive_path)?;
    Ok(archive_path)
}

pub fn build_install_plan(
    install_folder: &Path,
    pack_archive: &Path,
) -> Result<InstallPlan, InstallError> {
    let file = File::open(pack_archive)?;
    let mut archive = ZipArchive::new(file).map_err(|error| InstallError::Zip(error.to_string()))?;

    let stamp = backup_stamp();
    let backup_root = install_folder
        .join(BEPINEX_FOLDER)
        .join(MANAGER_BACKUP_FOLDER)
        .join(&stamp);

    let mut files = Vec::new();
    let mut events = Vec::new();
    let mut saw_pack_root = false;

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| InstallError::Zip(error.to_string()))?;
        let name = entry.name().to_string();
        if !name.starts_with(PACK_ROOT_PREFIX) {
            continue;
        }
        saw_pack_root = true;
        if name.ends_with('/') {
            continue;
        }

        let relative = PathBuf::from(
            name.strip_prefix(PACK_ROOT_PREFIX)
                .expect("prefix checked"),
        );
        ensure_safe_relative(&relative, &name)?;
        let destination = install_folder.join(&relative);
        let destination_exists = destination.exists();
        let bytes = entry.size();

        if destination_exists {
            let backup_to = backup_root.join(&relative);
            events.push(InstallEvent::WouldBackupFile {
                from: destination.clone(),
                to: backup_to,
            });
        } else if let Some(parent) = destination.parent() {
            if !parent.exists() {
                events.push(InstallEvent::WouldCreateDirectory {
                    path: parent.to_path_buf(),
                });
            }
        }

        events.push(InstallEvent::WouldWriteFile {
            path: destination.clone(),
            bytes,
        });

        if relative.as_os_str() == RUN_SCRIPT {
            events.push(InstallEvent::WouldSetExecutable {
                path: destination.clone(),
            });
        }

        files.push(PlannedFile {
            relative_path: relative,
            destination,
            destination_exists,
        });
    }

    if !saw_pack_root {
        return Err(InstallError::MissingPackRoot);
    }

    let version_path = manager_pack_version_path(install_folder);
    events.push(InstallEvent::WouldWritePackVersion {
        path: version_path,
        version: RECOMMENDED_PACK_VERSION.to_string(),
    });

    Ok(InstallPlan {
        archive_path: pack_archive.to_path_buf(),
        install_folder: install_folder.to_path_buf(),
        backup_root,
        files,
        events,
    })
}

pub fn apply_install_plan(plan: &InstallPlan) -> Result<Vec<InstallEvent>, InstallError> {
    let file = File::open(&plan.archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|error| InstallError::Zip(error.to_string()))?;

    let mut events = Vec::new();
    fs::create_dir_all(&plan.backup_root)?;
    events.push(InstallEvent::CreatedDirectory {
        path: plan.backup_root.clone(),
    });

    for planned in &plan.files {
        let entry_name = format!(
            "{PACK_ROOT_PREFIX}{}",
            planned.relative_path.to_string_lossy()
        );
        let mut entry = archive
            .by_name(&entry_name)
            .map_err(|error| InstallError::Zip(format!("{entry_name}: {error}")))?;

        if planned.destination_exists {
            let backup_to = plan.backup_root.join(&planned.relative_path);
            if let Some(parent) = backup_to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&planned.destination, &backup_to)?;
            events.push(InstallEvent::BackedUpFile {
                from: planned.destination.clone(),
                to: backup_to,
            });
        }

        if let Some(parent) = planned.destination.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
                events.push(InstallEvent::CreatedDirectory {
                    path: parent.to_path_buf(),
                });
            }
        }

        let mut output = File::create(&planned.destination)?;
        let written = io::copy(&mut entry, &mut output)?;
        events.push(InstallEvent::WroteFile {
            path: planned.destination.clone(),
            bytes: written,
        });

        if planned.relative_path.as_os_str() == RUN_SCRIPT {
            set_executable(&planned.destination)?;
            events.push(InstallEvent::SetExecutable {
                path: planned.destination.clone(),
            });
        }
    }

    let version_path = manager_pack_version_path(&plan.install_folder);
    if let Some(parent) = version_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&version_path, format!("{RECOMMENDED_PACK_VERSION}\n"))?;
    events.push(InstallEvent::WrotePackVersion {
        path: version_path,
        version: RECOMMENDED_PACK_VERSION.to_string(),
    });

    Ok(events)
}

fn cache_directory() -> Result<PathBuf, PackError> {
    if let Some(explicit) = std::env::var_os("SILKSONG_BEPINEX_CACHE") {
        return Ok(PathBuf::from(explicit));
    }
    Ok(crate::platform::cache_directory()
        .map_err(|_| PackError::CacheDirectoryUnavailable)?
        .join("bepinex"))
}

fn backup_stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{nanos}")
}

fn ensure_safe_relative(relative: &Path, entry_name: &str) -> Result<(), InstallError> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(component, Component::ParentDir | Component::RootDir)
        })
    {
        return Err(InstallError::UnsafeArchivePath {
            entry: entry_name.to_string(),
        });
    }
    Ok(())
}

fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
}

fn describe_event(event: &InstallEvent) -> String {
    match event {
        InstallEvent::WouldCreateDirectory { path } => {
            format!("would create directory {}", path.display())
        }
        InstallEvent::WouldWriteFile { path, bytes } => {
            format!("would write {} ({bytes} bytes)", path.display())
        }
        InstallEvent::WouldBackupFile { from, to } => {
            format!("would back up {} -> {}", from.display(), to.display())
        }
        InstallEvent::WouldSetExecutable { path } => {
            format!("would mark executable {}", path.display())
        }
        InstallEvent::WouldWritePackVersion { path, version } => {
            format!("would record pack version {version} at {}", path.display())
        }
        InstallEvent::CreatedDirectory { path } => format!("created directory {}", path.display()),
        InstallEvent::WroteFile { path, bytes } => {
            format!("wrote {} ({bytes} bytes)", path.display())
        }
        InstallEvent::BackedUpFile { from, to } => {
            format!("backed up {} -> {}", from.display(), to.display())
        }
        InstallEvent::SetExecutable { path } => format!("marked executable {}", path.display()),
        InstallEvent::WrotePackVersion { path, version } => {
            format!("recorded pack version {version} at {}", path.display())
        }
    }
}

fn describe_launch(outcome: &LaunchOptionsOutcome) -> String {
    match outcome {
        LaunchOptionsOutcome::AlreadyConfigured {
            launch_options,
            script,
        } => format!(
            "steam launch options already set ({launch_options}); launch script {}",
            describe_script(script)
        ),
        LaunchOptionsOutcome::Updated {
            localconfig_path,
            launch_options,
            script,
            steam_restart_recommended,
        } => {
            let mut text = format!(
                "wrote steam launch options in {} ({launch_options}); launch script {}",
                localconfig_path.display(),
                describe_script(script)
            );
            if *steam_restart_recommended {
                text.push_str("; fully restart steam once so the options stick");
            }
            text
        }
        LaunchOptionsOutcome::ManualPasteRequired {
            reason,
            launch_options,
            script,
        } => format!(
            "launch script {} ready, but steam launch options NOT set ({reason}). paste: {launch_options}",
            describe_script(script)
        ),
        LaunchOptionsOutcome::DryRun { launch_options } => {
            format!(
                "would create editable {SMM_LAUNCH_SCRIPT} and set steam launch options: {launch_options}"
            )
        }
    }
}

fn describe_script(script: &LaunchScriptAction) -> String {
    match script {
        LaunchScriptAction::Created { path } => format!("created {}", path.display()),
        LaunchScriptAction::AlreadyPresent { path } => {
            format!("left existing editable script at {}", path.display())
        }
        LaunchScriptAction::Reset { path } => format!("reset {}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-bepinex-install-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    fn write_pack_zip(path: &Path) {
        let file = File::create(path).expect("zip");
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

        zip.start_file("manifest.json", options).unwrap();
        zip.write_all(b"{}").unwrap();

        zip.start_file("BepInExPack/doorstop_config.ini", options)
            .unwrap();
        zip.write_all(b"enabled = true\n").unwrap();

        zip.start_file("BepInExPack/winhttp.dll", options).unwrap();
        zip.write_all(b"dll").unwrap();

        zip.start_file("BepInExPack/libdoorstop.so", options).unwrap();
        zip.write_all(b"so").unwrap();

        zip.start_file("BepInExPack/run_bepinex.sh", options).unwrap();
        zip.write_all(b"#!/bin/sh\n").unwrap();

        zip.start_file("BepInExPack/BepInEx/core/BepInEx.dll", options)
            .unwrap();
        zip.write_all(b"BepInEx 5.4.23.4").unwrap();

        zip.start_file("BepInExPack/BepInEx/core/BepInEx.Preloader.dll", options)
            .unwrap();
        zip.write_all(b"preloader").unwrap();

        zip.finish().unwrap();
    }

    #[test]
    fn dry_run_plan_lists_writes_without_touching_game_files() {
        let root = scratch("dry");
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        let archive = root.join("pack.zip");
        write_pack_zip(&archive);

        let plan = build_install_plan(&game, &archive).expect("plan");
        assert!(plan.events.iter().any(|event| matches!(
            event,
            InstallEvent::WouldWriteFile { path, .. } if path.ends_with("doorstop_config.ini")
        )));
        assert!(!game.join("doorstop_config.ini").exists());
        assert!(!game.join(BEPINEX_FOLDER).exists());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn apply_backs_up_existing_root_file_then_overwrites() {
        let root = scratch("apply");
        let game = root.join("game");
        fs::create_dir_all(&game).unwrap();
        fs::write(game.join("doorstop_config.ini"), b"old").unwrap();

        let archive = root.join("pack.zip");
        write_pack_zip(&archive);

        let plan = build_install_plan(&game, &archive).expect("plan");
        let events = apply_install_plan(&plan).expect("apply");

        assert_eq!(
            fs::read_to_string(game.join("doorstop_config.ini")).unwrap(),
            "enabled = true\n"
        );
        assert!(events.iter().any(|event| matches!(
            event,
            InstallEvent::BackedUpFile { from, .. } if from.ends_with("doorstop_config.ini")
        )));
        assert_eq!(
            fs::read_to_string(manager_pack_version_path(&game))
                .unwrap()
                .trim(),
            RECOMMENDED_PACK_VERSION
        );
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rejects_parent_directory_escape() {
        let relative = PathBuf::from("../outside");
        let error = ensure_safe_relative(&relative, "BepInExPack/../outside").unwrap_err();
        assert!(matches!(error, InstallError::UnsafeArchivePath { .. }));
    }
}
