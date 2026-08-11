use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG_FILE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug)]
pub enum LogError {
    HomeUnavailable,
    Io(io::Error),
    OpenFailed { path: PathBuf, detail: String },
}

impl std::fmt::Display for LogError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HomeUnavailable => write!(formatter, "HOME is unset; cannot create log directory"),
            Self::Io(error) => write!(formatter, "log io error: {error}"),
            Self::OpenFailed { path, detail } => {
                write!(
                    formatter,
                    "could not open log file {}: {detail}",
                    path.display()
                )
            }
        }
    }
}

impl std::error::Error for LogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for LogError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn data_directory() -> Result<PathBuf, LogError> {
    let home = std::env::var_os("HOME").ok_or(LogError::HomeUnavailable)?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("pollip"))
}

pub fn logs_directory() -> Result<PathBuf, LogError> {
    Ok(data_directory()?.join("logs"))
}

pub fn current_log_path() -> Option<PathBuf> {
    LOG_FILE.lock().ok()?.clone()
}

pub fn init_logging() -> Result<PathBuf, LogError> {
    let directory = logs_directory()?;
    fs::create_dir_all(&directory)?;
    let stamp = session_stamp();
    let path = directory.join(format!("session-{stamp}.log"));
    {
        let mut file = File::create(&path)?;
        writeln!(file, "pollip session started")?;
    }
    if let Ok(mut guard) = LOG_FILE.lock() {
        *guard = Some(path.clone());
    }
    Ok(path)
}

pub fn write_line(level: &str, message: &str) {
    let Ok(guard) = LOG_FILE.lock() else {
        return;
    };
    let Some(path) = guard.as_ref() else {
        return;
    };
    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(file, "[{level}] {message}");
}

pub fn info(message: impl AsRef<str>) {
    write_line("info", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write_line("error", message.as_ref());
}

pub fn open_current_log() -> Result<PathBuf, LogError> {
    let path = match current_log_path() {
        Some(path) => path,
        None => {
            let directory = logs_directory()?;
            fs::create_dir_all(&directory)?;
            directory
        }
    };
    open_path(&path)?;
    Ok(path)
}

fn open_path(path: &Path) -> Result<(), LogError> {
    match Command::new("xdg-open").arg(path).spawn() {
        Ok(_) => Ok(()),
        Err(error) => Err(LogError::OpenFailed {
            path: path.to_path_buf(),
            detail: error.to_string(),
        }),
    }
}

fn session_stamp() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{nanos}")
}
