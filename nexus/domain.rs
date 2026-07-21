pub const GAME_DOMAIN: &str = "hollowknightsilksong";
pub const API_BASE: &str = "https://api.nexusmods.com/v1";
pub const GRAPHQL_URL: &str = "https://api.nexusmods.com/v2/graphql";
pub const CATALOG_PAGE_SIZE: u32 = 80;
pub const CATALOG_CACHE_VERSION: u32 = 2;
pub const APP_NAME: &str = "silksong-mod-manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CACHE_MAX_AGE_SECS: u64 = 3_600;
pub const CACHE_ENV: &str = "SILKSONG_NEXUS_CACHE";
pub const META_FILE: &str = "nexus.json";
pub const KEY_FILE_NAME: &str = "nexus-key";
pub const NXM_DESKTOP_FILE: &str = "silksong-mod-manager-nxm.desktop";
pub const NXM_MIME: &str = "x-scheme-handler/nxm";

pub fn user_agent() -> String {
    format!("{APP_NAME}/{APP_VERSION}")
}
