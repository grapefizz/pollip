use super::inventory::{
    disabled_folder, plugins_folder, removed_folder, InstalledMod, DISABLED_DIR, PLUGINS_DIR,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum ModError {
    Io(std::io::Error),
    EntryMissing { path: PathBuf },
    DestinationTaken { path: PathBuf },
    OpenFailed { path: PathBuf, detail: String },
}

impl std::fmt::Display for ModError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            Self::EntryMissing { path } => {
                write!(formatter, "mod entry no longer exists at {}", path.display())
            }
            Self::DestinationTaken { path } => write!(
                formatter,
                "cannot move mod because {} already exists",
                path.display()
            ),
            Self::OpenFailed { path, detail } => {
                write!(
                    formatter,
                    "could not open {} in the file manager: {detail}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for ModError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ModError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn ensure_mod_folders(install_folder: &Path) -> Result<(), ModError> {
    fs::create_dir_all(plugins_folder(install_folder))?;
    fs::create_dir_all(disabled_folder(install_folder))?;
    fs::create_dir_all(removed_folder(install_folder))?;
    Ok(())
}

pub fn enable_mod(install_folder: &Path, installed: &InstalledMod) -> Result<PathBuf, ModError> {
    if installed.enabled {
        return Ok(installed.path.clone());
    }
    let destination = plugins_folder(install_folder).join(&installed.entry_name);
    move_entry(&installed.path, &destination)?;
    Ok(destination)
}

pub fn disable_mod(install_folder: &Path, installed: &InstalledMod) -> Result<PathBuf, ModError> {
    if !installed.enabled {
        return Ok(installed.path.clone());
    }
    fs::create_dir_all(disabled_folder(install_folder))?;
    let destination = disabled_folder(install_folder).join(&installed.entry_name);
    move_entry(&installed.path, &destination)?;
    Ok(destination)
}

pub fn remove_mod(install_folder: &Path, installed: &InstalledMod) -> Result<PathBuf, ModError> {
    let stamp = unique_stamp();
    let destination_root = removed_folder(install_folder).join(&stamp);
    fs::create_dir_all(&destination_root)?;
    let destination = destination_root.join(&installed.entry_name);
    move_entry(&installed.path, &destination)?;
    Ok(destination)
}

pub fn open_plugins_folder(install_folder: &Path) -> Result<PathBuf, ModError> {
    let folder = plugins_folder(install_folder);
    fs::create_dir_all(&folder)?;
    match crate::platform::open_path(&folder) {
        Ok(()) => Ok(folder),
        Err(error) => Err(ModError::OpenFailed {
            path: folder,
            detail: error.to_string(),
        }),
    }
}

pub fn describe_mod_locations(install_folder: &Path) -> String {
    format!(
        "enabled: {}\ndisabled: {}",
        install_folder.join(PLUGINS_DIR).display(),
        install_folder.join(DISABLED_DIR).display()
    )
}

fn move_entry(source: &Path, destination: &Path) -> Result<(), ModError> {
    if !source.exists() {
        return Err(ModError::EntryMissing {
            path: source.to_path_buf(),
        });
    }
    if destination.exists() {
        return Err(ModError::DestinationTaken {
            path: destination.to_path_buf(),
        });
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(source, destination)?;
    Ok(())
}

fn unique_stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::inventory::{scan_installed_mods, ModKind, MANIFEST_FILE};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-mods-manage-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write");
    }

    fn seed_folder_mod(root: &Path, entry_name: &str) -> InstalledMod {
        let path = root.join(PLUGINS_DIR).join(entry_name);
        write_file(
            &path.join(MANIFEST_FILE),
            r#"{ "name": "Sample", "version_number": "2.0.0" }"#,
        );
        write_file(&path.join("Sample.dll"), "dll");
        InstalledMod {
            entry_name: entry_name.to_string(),
            display_name: "Sample".to_string(),
            version: Some("2.0.0".to_string()),
            enabled: true,
            path,
            kind: ModKind::Folder,
            source: crate::mods::ModSource::Thunderstore,
        }
    }

    #[test]
    fn disable_then_enable_moves_between_folders() {
        let root = scratch("toggle");
        let installed = seed_folder_mod(&root, "sample_mod");

        let disabled_path = disable_mod(&root, &installed).expect("disable");
        assert!(disabled_path.starts_with(disabled_folder(&root)));
        assert!(!installed.path.exists());
        assert!(disabled_path.exists());

        let disabled = InstalledMod {
            enabled: false,
            path: disabled_path.clone(),
            ..installed.clone()
        };
        let enabled_path = enable_mod(&root, &disabled).expect("enable");
        assert!(enabled_path.starts_with(plugins_folder(&root)));
        assert!(enabled_path.exists());
        assert!(!disabled_path.exists());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn remove_moves_into_backup_instead_of_deleting() {
        let root = scratch("remove");
        let installed = seed_folder_mod(&root, "doomed_mod");
        let removed_path = remove_mod(&root, &installed).expect("remove");
        assert!(removed_path.starts_with(removed_folder(&root)));
        assert!(removed_path.exists());
        assert!(!installed.path.exists());
        assert!(scan_installed_mods(&root).expect("scan").is_empty());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn ensure_mod_folders_creates_expected_paths() {
        let root = scratch("folders");
        ensure_mod_folders(&root).expect("ensure");
        assert!(plugins_folder(&root).is_dir());
        assert!(disabled_folder(&root).is_dir());
        assert!(removed_folder(&root).is_dir());
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn stage1_style_fake_install_full_flow() {
        let library = scratch("stage1-fake");
        let install = library.join("Hollow Knight Silksong");
        fs::create_dir_all(install.join(PLUGINS_DIR)).expect("plugins");
        fs::write(install.join("Hollow Knight Silksong"), []).expect("executable");

        write_file(
            &install
                .join(PLUGINS_DIR)
                .join("silk_radar")
                .join(MANIFEST_FILE),
            r#"{ "name": "Silk Radar", "version_number": "1.0.0" }"#,
        );
        write_file(
            &install.join(PLUGINS_DIR).join("silk_radar").join("SilkRadar.dll"),
            "dll",
        );
        write_file(&install.join(PLUGINS_DIR).join("QuickSlot.dll"), "dll");
        write_file(
            &install
                .join(PLUGINS_DIR)
                .join("health_bars")
                .join("HealthBars.dll"),
            "dll",
        );

        let listed = scan_installed_mods(&install).expect("scan");
        assert_eq!(listed.len(), 3);

        let quick = listed
            .iter()
            .find(|entry| entry.entry_name == "QuickSlot.dll")
            .expect("quickslot")
            .clone();
        let disabled_path = disable_mod(&install, &quick).expect("disable");
        assert!(disabled_path.starts_with(disabled_folder(&install)));

        let health = listed
            .iter()
            .find(|entry| entry.entry_name == "health_bars")
            .expect("health")
            .clone();
        let removed_path = remove_mod(&install, &health).expect("remove");
        assert!(removed_path.starts_with(removed_folder(&install)));
        assert!(removed_path.join("HealthBars.dll").is_file());

        let remaining = scan_installed_mods(&install).expect("rescan");
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().any(|entry| entry.display_name == "Silk Radar" && entry.enabled));
        assert!(remaining.iter().any(|entry| entry.entry_name == "QuickSlot.dll" && !entry.enabled));

        fs::remove_dir_all(library).ok();
    }
}
