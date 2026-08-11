use std::fmt;

pub const RECOMMENDED_PACK_NAMESPACE: &str = "silksong_modding";
pub const RECOMMENDED_PACK_NAME: &str = "BepInExPack_Silksong";
pub const RECOMMENDED_PACK_VERSION: &str = "1.0.3";
pub const RECOMMENDED_PACK_FULL_NAME: &str = "silksong_modding-BepInExPack_Silksong-1.0.3";
pub const RECOMMENDED_BEPINEX_VERSION: &str = "5.4.23.4";
pub const PACK_ROOT_PREFIX: &str = "BepInExPack/";
pub const MANAGER_PACK_VERSION_FILE: &str = ".manager_pack_version";
pub const MANAGER_BACKUP_FOLDER: &str = ".manager_backup";
pub const BEPINEX_FOLDER: &str = "BepInEx";
pub const CORE_ASSEMBLY: &str = "BepInEx/core/BepInEx.dll";
pub const PRELOADER_ASSEMBLY: &str = "BepInEx/core/BepInEx.Preloader.dll";
pub const DOORSTOP_CONFIG: &str = "doorstop_config.ini";
pub const WINHTTP_PROXY: &str = "winhttp.dll";
pub const LINUX_DOORSTOP: &str = "libdoorstop.so";
pub const MACOS_DOORSTOP: &str = "libdoorstop.dylib";
pub const RUN_SCRIPT: &str = "run_bepinex.sh";
pub const DOORSTOP_VERSION_FILE: &str = ".doorstop_version";
pub const LOG_OUTPUT: &str = "BepInEx/LogOutput.log";
pub const PLUGINS_FOLDER: &str = "BepInEx/plugins";
pub const SMM_LAUNCH_SCRIPT: &str = "smm_launch.sh";
#[cfg(not(target_os = "macos"))]
pub const STEAM_LAUNCH_OPTIONS: &str = "./smm_launch.sh %command%";

pub fn pack_download_url() -> String {
    format!(
        "https://thunderstore.io/package/download/{RECOMMENDED_PACK_NAMESPACE}/{RECOMMENDED_PACK_NAME}/{RECOMMENDED_PACK_VERSION}/"
    )
}

pub fn pack_cache_file_name() -> String {
    format!("{RECOMMENDED_PACK_FULL_NAME}.zip")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackError {
    CacheDirectoryUnavailable,
    DownloadFailed { detail: String },
}

impl fmt::Display for PackError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CacheDirectoryUnavailable => {
                write!(formatter, "could not resolve a cache directory for bepinex downloads")
            }
            Self::DownloadFailed { detail } => {
                write!(formatter, "failed to download {RECOMMENDED_PACK_FULL_NAME}: {detail}")
            }
        }
    }
}

impl std::error::Error for PackError {}
