use super::pack::{
    CORE_ASSEMBLY, DOORSTOP_CONFIG, DOORSTOP_VERSION_FILE, LINUX_DOORSTOP,
    MANAGER_PACK_VERSION_FILE, PRELOADER_ASSEMBLY, RECOMMENDED_BEPINEX_VERSION,
    RECOMMENDED_PACK_VERSION, RUN_SCRIPT, WINHTTP_PROXY, BEPINEX_FOLDER,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BepinexStatus {
    NotInstalled,
    InstalledCurrent {
        pack_version: Option<String>,
        bepinex_version: Option<String>,
    },
    NeedsUpdate {
        pack_version: Option<String>,
        bepinex_version: Option<String>,
        recommended_pack_version: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BepinexPresence {
    pub bepinex_folder: bool,
    pub core_assembly: bool,
    pub preloader_assembly: bool,
    pub doorstop_config: bool,
    pub winhttp_proxy: bool,
    pub linux_doorstop: bool,
    pub run_script: bool,
    pub doorstop_version_file: bool,
    pub pack_version: Option<String>,
    pub bepinex_version: Option<String>,
}

impl BepinexPresence {
    pub fn looks_installed(&self) -> bool {
        self.bepinex_folder
            && self.core_assembly
            && self.preloader_assembly
            && self.doorstop_config
            && self.winhttp_proxy
            && self.linux_doorstop
            && self.run_script
    }

    pub fn into_status(self) -> BepinexStatus {
        if !self.looks_installed() {
            return BepinexStatus::NotInstalled;
        }

        let pack_is_current = self
            .pack_version
            .as_deref()
            .map(|version| version == RECOMMENDED_PACK_VERSION)
            .unwrap_or(false);
        let bepinex_is_current = self
            .bepinex_version
            .as_deref()
            .map(|version| version == RECOMMENDED_BEPINEX_VERSION)
            .unwrap_or(false);

        if pack_is_current || (self.pack_version.is_none() && bepinex_is_current) {
            BepinexStatus::InstalledCurrent {
                pack_version: self.pack_version,
                bepinex_version: self.bepinex_version,
            }
        } else {
            BepinexStatus::NeedsUpdate {
                pack_version: self.pack_version,
                bepinex_version: self.bepinex_version,
                recommended_pack_version: RECOMMENDED_PACK_VERSION.to_string(),
            }
        }
    }
}

pub fn inspect_bepinex(install_folder: &Path) -> BepinexStatus {
    presence_in(install_folder).into_status()
}

pub fn presence_in(install_folder: &Path) -> BepinexPresence {
    let bepinex_folder = install_folder.join(BEPINEX_FOLDER).is_dir();
    let core_assembly = install_folder.join(CORE_ASSEMBLY).is_file();
    let preloader_assembly = install_folder.join(PRELOADER_ASSEMBLY).is_file();
    let doorstop_config = install_folder.join(DOORSTOP_CONFIG).is_file();
    let winhttp_proxy = install_folder.join(WINHTTP_PROXY).is_file();
    let linux_doorstop = install_folder.join(LINUX_DOORSTOP).is_file();
    let run_script = install_folder.join(RUN_SCRIPT).is_file();
    let doorstop_version_file = install_folder.join(DOORSTOP_VERSION_FILE).is_file();
    let pack_version = read_pack_version(install_folder);
    let bepinex_version = read_bepinex_version_hint(install_folder);

    BepinexPresence {
        bepinex_folder,
        core_assembly,
        preloader_assembly,
        doorstop_config,
        winhttp_proxy,
        linux_doorstop,
        run_script,
        doorstop_version_file,
        pack_version,
        bepinex_version,
    }
}

pub fn manager_pack_version_path(install_folder: &Path) -> PathBuf {
    install_folder
        .join(BEPINEX_FOLDER)
        .join(MANAGER_PACK_VERSION_FILE)
}

fn read_pack_version(install_folder: &Path) -> Option<String> {
    let path = manager_pack_version_path(install_folder);
    fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn read_bepinex_version_hint(install_folder: &Path) -> Option<String> {
    let assembly = install_folder.join(CORE_ASSEMBLY);
    let bytes = fs::read(assembly).ok()?;
    find_version_string(&bytes, RECOMMENDED_BEPINEX_VERSION)
        .or_else(|| find_dotted_version_near_bepinex(&bytes))
}

fn find_version_string(bytes: &[u8], needle: &str) -> Option<String> {
    let needle_bytes = needle.as_bytes();
    bytes
        .windows(needle_bytes.len())
        .any(|window| window == needle_bytes)
        .then(|| needle.to_string())
}

fn find_dotted_version_near_bepinex(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    for candidate in text.split(|ch: char| !ch.is_ascii_digit() && ch != '.') {
        if is_plausible_bepinex_version(candidate) {
            return Some(candidate.to_string());
        }
    }
    None
}

fn is_plausible_bepinex_version(candidate: &str) -> bool {
    let parts: Vec<&str> = candidate.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|part| {
        !part.is_empty()
            && part.len() <= 3
            && part.chars().all(|ch| ch.is_ascii_digit())
    }) && candidate.starts_with("5.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-bepinex-status-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    fn write_file(path: &Path, contents: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write");
    }

    fn seed_complete_install(root: &Path) {
        write_file(&root.join(CORE_ASSEMBLY), b"BepInEx 5.4.23.4 loader");
        write_file(&root.join(PRELOADER_ASSEMBLY), b"preloader");
        write_file(&root.join(DOORSTOP_CONFIG), b"enabled = true");
        write_file(&root.join(WINHTTP_PROXY), b"dll");
        write_file(&root.join(LINUX_DOORSTOP), b"so");
        write_file(&root.join(RUN_SCRIPT), b"#!/bin/sh");
        write_file(&root.join(DOORSTOP_VERSION_FILE), b"4.4.1");
        write_file(
            &manager_pack_version_path(root),
            RECOMMENDED_PACK_VERSION.as_bytes(),
        );
    }

    #[test]
    fn missing_install_is_not_installed() {
        let root = scratch("missing");
        assert_eq!(inspect_bepinex(&root), BepinexStatus::NotInstalled);
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn complete_current_install_reports_current() {
        let root = scratch("current");
        seed_complete_install(&root);
        match inspect_bepinex(&root) {
            BepinexStatus::InstalledCurrent { pack_version, .. } => {
                assert_eq!(pack_version.as_deref(), Some(RECOMMENDED_PACK_VERSION));
            }
            other => panic!("expected current, got {other:?}"),
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn outdated_pack_marker_needs_update() {
        let root = scratch("outdated");
        seed_complete_install(&root);
        write_file(&manager_pack_version_path(&root), b"1.0.0");
        match inspect_bepinex(&root) {
            BepinexStatus::NeedsUpdate {
                recommended_pack_version,
                ..
            } => {
                assert_eq!(recommended_pack_version, RECOMMENDED_PACK_VERSION);
            }
            other => panic!("expected needs update, got {other:?}"),
        }
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn incomplete_install_counts_as_not_installed() {
        let root = scratch("incomplete");
        write_file(&root.join(CORE_ASSEMBLY), b"BepInEx 5.4.23.4");
        assert_eq!(inspect_bepinex(&root), BepinexStatus::NotInstalled);
        fs::remove_dir_all(root).ok();
    }
}
