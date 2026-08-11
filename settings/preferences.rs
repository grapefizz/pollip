use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum PreferencesError {
    HomeUnavailable,
    Decode(String),
    Io(io::Error),
}

impl std::fmt::Display for PreferencesError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HomeUnavailable => {
                write!(formatter, "HOME is unset; cannot store preferences")
            }
            Self::Decode(detail) => write!(formatter, "preferences json error: {detail}"),
            Self::Io(error) => write!(formatter, "preferences io error: {error}"),
        }
    }
}

impl std::error::Error for PreferencesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for PreferencesError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Preferences {
    pub install_folder: Option<PathBuf>,
    pub setup_complete: bool,
}

pub fn preferences_path() -> Result<PathBuf, PreferencesError> {
    let home = std::env::var_os("HOME").ok_or(PreferencesError::HomeUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("pollip")
        .join("preferences.json"))
}

pub fn load_preferences() -> Result<Preferences, PreferencesError> {
    let path = preferences_path()?;
    if !path.is_file() {
        return Ok(Preferences::default());
    }
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text).map_err(|error| PreferencesError::Decode(error.to_string()))
}

pub fn save_preferences(preferences: &Preferences) -> Result<(), PreferencesError> {
    let path = preferences_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(preferences)
        .map_err(|error| PreferencesError::Decode(error.to_string()))?;
    let temporary = path.with_extension("json.partial");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(temporary, path)?;
    Ok(())
}

pub fn remember_install_folder(folder: &Path) -> Result<(), PreferencesError> {
    let mut preferences = load_preferences().unwrap_or_default();
    preferences.install_folder = Some(folder.to_path_buf());
    save_preferences(&preferences)
}

pub fn mark_setup_complete() -> Result<(), PreferencesError> {
    let mut preferences = load_preferences().unwrap_or_default();
    preferences.setup_complete = true;
    save_preferences(&preferences)
}
