use std::fs;
use std::path::{Path, PathBuf};

use super::model::ConfigDocument;
use super::parse::{parse_config_text, serialize_config, ParseError};

pub const BEPINEX_CONFIG_DIR: &str = "BepInEx/config";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFileSummary {
    pub path: PathBuf,
    pub file_name: String,
    pub display_name: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    NotADirectory { path: PathBuf },
    Parse(ParseError),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
            Self::NotADirectory { path } => {
                write!(formatter, "expected a directory: {}", path.display())
            }
            Self::Parse(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<std::io::Error> for ConfigError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ParseError> for ConfigError {
    fn from(error: ParseError) -> Self {
        Self::Parse(error)
    }
}

pub fn bepinex_config_folder(install_folder: &Path) -> PathBuf {
    install_folder.join(BEPINEX_CONFIG_DIR)
}

pub fn scan_config_files(install_folder: &Path) -> Result<Vec<ConfigFileSummary>, ConfigError> {
    let folder = bepinex_config_folder(install_folder);
    if !folder.exists() {
        return Ok(Vec::new());
    }
    if !folder.is_dir() {
        return Err(ConfigError::NotADirectory { path: folder });
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&folder)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(extension) = path.extension() else {
            continue;
        };
        if !extension.eq_ignore_ascii_case("cfg") {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let display_name = display_name_from_file_name(&file_name);
        files.push(ConfigFileSummary {
            path,
            file_name,
            display_name,
        });
    }

    files.sort_by(|left, right| {
        left.display_name
            .to_lowercase()
            .cmp(&right.display_name.to_lowercase())
            .then_with(|| left.file_name.cmp(&right.file_name))
    });
    Ok(files)
}

pub fn load_config_file(path: &Path) -> Result<ConfigDocument, ConfigError> {
    let text = fs::read_to_string(path)?;
    Ok(parse_config_text(&text)?)
}

pub fn save_config_file(path: &Path, document: &ConfigDocument) -> Result<(), ConfigError> {
    let text = serialize_config(document);
    fs::write(path, text)?;
    Ok(())
}

fn display_name_from_file_name(file_name: &str) -> String {
    file_name
        .strip_suffix(".cfg")
        .or_else(|| file_name.strip_suffix(".CFG"))
        .unwrap_or(file_name)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_directory(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-config-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("create scratch");
        directory
    }

    #[test]
    fn scans_only_cfg_files() {
        let install = scratch_directory("scan");
        let config_dir = bepinex_config_folder(&install);
        fs::create_dir_all(&config_dir).expect("config dir");
        fs::write(config_dir.join("com.example.mod.cfg"), "[A]\nKey = 1\n").expect("write cfg");
        fs::write(config_dir.join("notes.txt"), "ignore").expect("write txt");
        fs::create_dir_all(config_dir.join("nested")).expect("nested");

        let files = scan_config_files(&install).expect("scan");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_name, "com.example.mod.cfg");
        assert_eq!(files[0].display_name, "com.example.mod");

        fs::remove_dir_all(install).ok();
    }

    #[test]
    fn load_and_save_round_trip() {
        let install = scratch_directory("roundtrip");
        let config_dir = bepinex_config_folder(&install);
        fs::create_dir_all(&config_dir).expect("config dir");
        let path = config_dir.join("demo.cfg");
        let original = "[General]\n\n# Default value: yes\nFlag = no\n";
        fs::write(&path, original).expect("write");

        let mut document = load_config_file(&path).expect("load");
        let setting = document.editable_settings().remove(0);
        document.set_value(setting.line_index, "yes");
        save_config_file(&path, &document).expect("save");

        let saved = fs::read_to_string(&path).expect("read back");
        assert_eq!(saved, "[General]\n\n# Default value: yes\nFlag = yes\n");

        fs::remove_dir_all(install).ok();
    }

    #[test]
    fn load_surfaces_parse_errors() {
        let install = scratch_directory("badparse");
        let config_dir = bepinex_config_folder(&install);
        fs::create_dir_all(&config_dir).expect("config dir");
        let path = config_dir.join("broken.cfg");
        fs::write(&path, "[General]\nnot valid\n").expect("write");

        let error = load_config_file(&path).expect_err("parse must fail");
        let message = error.to_string();
        assert!(message.contains("line 2"));

        fs::remove_dir_all(install).ok();
    }
}
