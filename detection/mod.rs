mod steam;

use std::fmt;
use std::path::{Path, PathBuf};

pub use steam::{candidate_steam_roots, default_steam_common_folders};

pub const SILKSONG_APP_ID: u32 = 1_030_300;
pub const SILKSONG_FOLDER_NAME: &str = "Hollow Knight Silksong";
pub const NATIVE_LINUX_EXECUTABLE: &str = "Hollow Knight Silksong";
pub const WINDOWS_EXECUTABLE: &str = "Hollow Knight Silksong.exe";
pub const MACOS_APPLICATION: &str = "Hollow Knight Silksong.app";
pub const DETECT_ROOT_ENV: &str = "SILKSONG_DETECT_ROOT";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildKind {
    NativeLinux,
    NativeMacOS,
    Proton,
}

impl fmt::Display for BuildKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NativeLinux => write!(formatter, "native linux"),
            Self::NativeMacOS => write!(formatter, "native macOS"),
            Self::Proton => write!(formatter, "proton"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SilksongInstall {
    pub install_folder: PathBuf,
    pub executable: PathBuf,
    pub build_kind: BuildKind,
    pub proton_prefix: Option<PathBuf>,
}

#[derive(Debug)]
pub enum DetectionError {
    InstallNotFound { searched: Vec<PathBuf> },
    ExecutableMissing { install_folder: PathBuf },
    PathMissing { path: PathBuf },
    NotADirectory { path: PathBuf },
    HomeUnavailable,
    Io(std::io::Error),
}

impl fmt::Display for DetectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InstallNotFound { searched } => {
                write!(
                    formatter,
                    "could not find \"{SILKSONG_FOLDER_NAME}\" under any steam common folder"
                )?;
                if searched.is_empty() {
                    write!(formatter, " (no steam library paths were available to search)")
                } else {
                    write!(formatter, ". searched:")?;
                    for folder in searched {
                        write!(formatter, "\n  - {}", folder.display())?;
                    }
                    Ok(())
                }
            }
            Self::ExecutableMissing { install_folder } => write!(
                formatter,
                "found install at {} but none of \"{NATIVE_LINUX_EXECUTABLE}\", \"{MACOS_APPLICATION}\", or \"{WINDOWS_EXECUTABLE}\" is present",
                install_folder.display()
            ),
            Self::PathMissing { path } => {
                write!(formatter, "path does not exist: {}", path.display())
            }
            Self::NotADirectory { path } => {
                write!(formatter, "expected a directory: {}", path.display())
            }
            Self::HomeUnavailable => {
                write!(formatter, "HOME is unset; cannot resolve default steam paths")
            }
            Self::Io(error) => write!(formatter, "filesystem error: {error}"),
        }
    }
}

impl std::error::Error for DetectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

pub fn detect() -> Result<SilksongInstall, DetectionError> {
    if let Some(override_root) = detect_root_override() {
        return find_under_common_folders(&[override_root]);
    }
    find_under_common_folders(&default_steam_common_folders()?)
}

pub fn detect_root_override() -> Option<PathBuf> {
    std::env::var_os(DETECT_ROOT_ENV).map(PathBuf::from)
}

pub fn find_under_common_folders(
    common_folders: &[PathBuf],
) -> Result<SilksongInstall, DetectionError> {
    let mut searched = Vec::new();

    for common_folder in common_folders {
        searched.push(common_folder.clone());
        let candidate = common_folder.join(SILKSONG_FOLDER_NAME);
        if candidate.is_dir() {
            return inspect_install_folder(&candidate);
        }
    }

    Err(DetectionError::InstallNotFound { searched })
}

pub fn inspect_install_folder(install_folder: &Path) -> Result<SilksongInstall, DetectionError> {
    if !install_folder.exists() {
        return Err(DetectionError::PathMissing {
            path: install_folder.to_path_buf(),
        });
    }
    if !install_folder.is_dir() {
        return Err(DetectionError::NotADirectory {
            path: install_folder.to_path_buf(),
        });
    }

    let native_executable = install_folder.join(NATIVE_LINUX_EXECUTABLE);
    let macos_application = install_folder.join(MACOS_APPLICATION);
    let windows_executable = install_folder.join(WINDOWS_EXECUTABLE);

    let (executable, build_kind) = if windows_executable.is_file() {
        (windows_executable, BuildKind::Proton)
    } else if macos_application.is_dir() {
        (macos_application, BuildKind::NativeMacOS)
    } else if native_executable.is_file() {
        (native_executable, BuildKind::NativeLinux)
    } else {
        return Err(DetectionError::ExecutableMissing {
            install_folder: install_folder.to_path_buf(),
        });
    };

    let proton_prefix = match build_kind {
        BuildKind::Proton => locate_proton_prefix(install_folder),
        BuildKind::NativeLinux | BuildKind::NativeMacOS => None,
    };

    Ok(SilksongInstall {
        install_folder: install_folder.to_path_buf(),
        executable,
        build_kind,
        proton_prefix,
    })
}

fn locate_proton_prefix(install_folder: &Path) -> Option<PathBuf> {
    let steamapps = install_folder.parent()?.parent()?;
    let compat_entry = steamapps
        .join("compatdata")
        .join(SILKSONG_APP_ID.to_string());
    prefer_pfx_directory(&compat_entry).or_else(|| search_proton_prefix_across_libraries())
}

fn search_proton_prefix_across_libraries() -> Option<PathBuf> {
    let Ok(common_folders) = default_steam_common_folders() else {
        return None;
    };

    for common_folder in common_folders {
        let steamapps = common_folder.parent()?;
        let compat_entry = steamapps
            .join("compatdata")
            .join(SILKSONG_APP_ID.to_string());
        if let Some(prefix) = prefer_pfx_directory(&compat_entry) {
            return Some(prefix);
        }
    }

    None
}

fn prefer_pfx_directory(compat_entry: &Path) -> Option<PathBuf> {
    let pfx = compat_entry.join("pfx");
    if pfx.is_dir() {
        return Some(pfx);
    }
    if compat_entry.is_dir() {
        return Some(compat_entry.to_path_buf());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch_directory(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("silksong-detect-{label}-{nanos}"));
        fs::create_dir_all(&directory).expect("create scratch");
        directory
    }

    fn write_empty_file(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, []).expect("write file");
    }

    #[test]
    fn inspects_native_linux_install() {
        let root = scratch_directory("native");
        let install = root.join(SILKSONG_FOLDER_NAME);
        write_empty_file(&install.join(NATIVE_LINUX_EXECUTABLE));

        let found = inspect_install_folder(&install).expect("native install");
        assert_eq!(found.build_kind, BuildKind::NativeLinux);
        assert_eq!(found.executable, install.join(NATIVE_LINUX_EXECUTABLE));
        assert!(found.proton_prefix.is_none());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn inspects_macos_application_bundle() {
        let root = scratch_directory("macos");
        fs::create_dir_all(root.join(MACOS_APPLICATION)).expect("application bundle");

        let found = inspect_install_folder(&root).expect("macos install");
        assert_eq!(found.executable, root.join(MACOS_APPLICATION));
        assert_eq!(found.build_kind, BuildKind::NativeMacOS);
        assert_eq!(found.proton_prefix, None);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn inspects_proton_install_and_prefix() {
        let library = scratch_directory("proton");
        let install = library
            .join("steamapps")
            .join("common")
            .join(SILKSONG_FOLDER_NAME);
        write_empty_file(&install.join(WINDOWS_EXECUTABLE));

        let pfx = library
            .join("steamapps")
            .join("compatdata")
            .join(SILKSONG_APP_ID.to_string())
            .join("pfx");
        fs::create_dir_all(&pfx).expect("create pfx");

        let found = inspect_install_folder(&install).expect("proton install");
        assert_eq!(found.build_kind, BuildKind::Proton);
        assert_eq!(found.executable, install.join(WINDOWS_EXECUTABLE));
        assert_eq!(found.proton_prefix.as_deref(), Some(pfx.as_path()));

        fs::remove_dir_all(library).ok();
    }

    #[test]
    fn finds_install_under_custom_common_root() {
        let common = scratch_directory("common");
        let install = common.join(SILKSONG_FOLDER_NAME);
        write_empty_file(&install.join(NATIVE_LINUX_EXECUTABLE));

        let found = find_under_common_folders(&[common.clone()]).expect("find under common");
        assert_eq!(found.install_folder, install);

        fs::remove_dir_all(common).ok();
    }

    #[test]
    fn prefers_windows_executable_when_both_present() {
        let root = scratch_directory("both");
        let install = root.join(SILKSONG_FOLDER_NAME);
        write_empty_file(&install.join(NATIVE_LINUX_EXECUTABLE));
        write_empty_file(&install.join(WINDOWS_EXECUTABLE));

        let found = inspect_install_folder(&install).expect("both present");
        assert_eq!(found.build_kind, BuildKind::Proton);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn reports_missing_install_clearly() {
        let empty = scratch_directory("empty");
        let error = find_under_common_folders(&[empty.clone()]).expect_err("should miss");
        let message = error.to_string();
        assert!(message.contains(SILKSONG_FOLDER_NAME));
        assert!(message.contains(&empty.display().to_string()));
        fs::remove_dir_all(empty).ok();
    }
}
