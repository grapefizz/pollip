use std::fs;
use std::path::{Path, PathBuf};

pub const PLUGINS_DIR: &str = "BepInEx/plugins";
pub const DISABLED_DIR: &str = "BepInEx/plugins_disabled";
pub const REMOVED_DIR: &str = "BepInEx/.manager_removed";
pub const MANIFEST_FILE: &str = "manifest.json";
pub const NEXUS_META_FILE: &str = "nexus.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModKind {
    Folder,
    Assembly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModSource {
    Thunderstore,
    Nexus,
    Unknown,
}

impl ModSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Thunderstore => "thunderstore",
            Self::Nexus => "nexus",
            Self::Unknown => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledMod {
    pub entry_name: String,
    pub display_name: String,
    pub version: Option<String>,
    pub enabled: bool,
    pub path: PathBuf,
    pub kind: ModKind,
    pub source: ModSource,
}

pub fn plugins_folder(install_folder: &Path) -> PathBuf {
    install_folder.join(PLUGINS_DIR)
}

pub fn disabled_folder(install_folder: &Path) -> PathBuf {
    install_folder.join(DISABLED_DIR)
}

pub fn removed_folder(install_folder: &Path) -> PathBuf {
    install_folder.join(REMOVED_DIR)
}

pub fn scan_installed_mods(install_folder: &Path) -> Result<Vec<InstalledMod>, std::io::Error> {
    let mut mods = Vec::new();
    collect_from_folder(&plugins_folder(install_folder), true, &mut mods)?;
    collect_from_folder(&disabled_folder(install_folder), false, &mut mods)?;
    mods.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.entry_name.cmp(&right.entry_name))
    });
    Ok(mods)
}

fn collect_from_folder(
    folder: &Path,
    enabled: bool,
    mods: &mut Vec<InstalledMod>,
) -> Result<(), std::io::Error> {
    if !folder.exists() {
        return Ok(());
    }
    if !folder.is_dir() {
        return Err(std::io::Error::other(format!(
            "expected a directory at {}",
            folder.display()
        )));
    }

    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        if file_name.starts_with('.') {
            continue;
        }

        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            let (display_name, version) = metadata_from_folder(&path, &file_name);
            let source = detect_source(&path, &file_name);
            mods.push(InstalledMod {
                entry_name: file_name,
                display_name,
                version,
                enabled,
                path,
                kind: ModKind::Folder,
                source,
            });
        } else if metadata.is_file() && is_assembly(&file_name) {
            let display_name = display_name_from_assembly(&file_name);
            mods.push(InstalledMod {
                entry_name: file_name,
                display_name,
                version: None,
                enabled,
                path,
                kind: ModKind::Assembly,
                source: ModSource::Unknown,
            });
        }
    }

    Ok(())
}

fn is_assembly(file_name: &str) -> bool {
    Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
}

fn display_name_from_assembly(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name)
        .to_string()
}

fn metadata_from_folder(folder: &Path, fallback_name: &str) -> (String, Option<String>) {
    let manifest_path = folder.join(MANIFEST_FILE);
    if let Some((name, version)) = read_manifest(&manifest_path) {
        return (name, version);
    }
    (fallback_name.to_string(), None)
}

fn detect_source(folder: &Path, entry_name: &str) -> ModSource {
    if folder.join(NEXUS_META_FILE).is_file() || entry_name.starts_with("nexus-") {
        return ModSource::Nexus;
    }
    if entry_name.contains('-') && folder.join(MANIFEST_FILE).is_file() {
        return ModSource::Thunderstore;
    }
    ModSource::Unknown
}

fn read_manifest(path: &Path) -> Option<(String, Option<String>)> {
    let text = fs::read_to_string(path).ok()?;
    let name = json_string_field(&text, "name")?;
    let version = json_string_field(&text, "version_number");
    Some((name, version))
}

fn json_string_field(text: &str, key: &str) -> Option<String> {
    let key_pattern = format!("\"{key}\"");
    let key_offset = text.find(&key_pattern)?;
    let after_key = &text[key_offset + key_pattern.len()..];
    let colon_offset = after_key.find(':')?;
    let after_colon = after_key[colon_offset + 1..].trim_start();
    if !after_colon.starts_with('"') {
        return None;
    }
    let value_body = &after_colon[1..];
    let mut output = String::new();
    let mut characters = value_body.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => {
                let escaped = characters.next()?;
                output.push(escaped);
            }
            '"' => return Some(output),
            other => output.push(other),
        }
    }
    None
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
        let directory = std::env::temp_dir().join(format!("silksong-mods-scan-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("scratch");
        directory
    }

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("parent");
        }
        fs::write(path, contents).expect("write");
    }

    #[test]
    fn scans_enabled_folder_mod_with_manifest() {
        let root = scratch("manifest");
        write_file(
            &root.join(PLUGINS_DIR).join("demo_mod").join(MANIFEST_FILE),
            r#"{ "name": "Demo Mod", "version_number": "1.2.3" }"#,
        );
        write_file(
            &root.join(PLUGINS_DIR).join("demo_mod").join("DemoMod.dll"),
            "dll",
        );

        let mods = scan_installed_mods(&root).expect("scan");
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].display_name, "Demo Mod");
        assert_eq!(mods[0].version.as_deref(), Some("1.2.3"));
        assert!(mods[0].enabled);
        assert_eq!(mods[0].kind, ModKind::Folder);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn scans_loose_assembly_and_disabled_folder() {
        let root = scratch("mixed");
        write_file(&root.join(PLUGINS_DIR).join("LooseMod.dll"), "dll");
        write_file(
            &root
                .join(DISABLED_DIR)
                .join("parked_mod")
                .join(MANIFEST_FILE),
            r#"{ "name": "Parked", "version_number": "0.1.0" }"#,
        );

        let mods = scan_installed_mods(&root).expect("scan");
        assert_eq!(mods.len(), 2);
        let loose = mods.iter().find(|entry| entry.entry_name == "LooseMod.dll");
        let parked = mods.iter().find(|entry| entry.entry_name == "parked_mod");
        assert!(loose.is_some_and(|entry| entry.enabled && entry.kind == ModKind::Assembly));
        assert!(parked.is_some_and(|entry| !entry.enabled && entry.display_name == "Parked"));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn missing_plugins_folder_yields_empty_list() {
        let root = scratch("empty");
        let mods = scan_installed_mods(&root).expect("scan");
        assert!(mods.is_empty());
        fs::remove_dir_all(root).ok();
    }
}
