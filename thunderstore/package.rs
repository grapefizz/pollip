use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiPackage {
    pub name: String,
    pub full_name: String,
    pub owner: String,
    pub date_updated: String,
    #[serde(default)]
    pub is_deprecated: bool,
    pub versions: Vec<ApiVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiVersion {
    pub name: String,
    pub full_name: String,
    #[serde(default)]
    pub description: String,
    pub version_number: String,
    #[serde(default)]
    pub downloads: u64,
    pub download_url: String,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemotePackage {
    pub name: String,
    pub full_name: String,
    pub owner: String,
    pub description: String,
    pub version: String,
    pub version_full_name: String,
    pub downloads: u64,
    pub download_url: String,
    pub icon_url: String,
    pub dependencies: Vec<String>,
    pub date_updated: String,
    pub is_deprecated: bool,
}

impl RemotePackage {
    pub fn from_api(package: ApiPackage) -> Option<Self> {
        let latest = package.versions.into_iter().next()?;
        let icon_url = if latest.icon.trim().is_empty() {
            format!(
                "https://gcdn.thunderstore.io/live/repository/icons/{}.png",
                latest.full_name
            )
        } else {
            latest.icon
        };
        Some(Self {
            name: package.name,
            full_name: package.full_name,
            owner: package.owner,
            description: latest.description,
            version: latest.version_number,
            version_full_name: latest.full_name,
            downloads: latest.downloads,
            download_url: latest.download_url,
            icon_url,
            dependencies: latest.dependencies,
            date_updated: package.date_updated,
            is_deprecated: package.is_deprecated,
        })
    }

    pub fn matches_search(&self, query: &str) -> bool {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        self.name.to_lowercase().contains(&needle)
            || self.full_name.to_lowercase().contains(&needle)
            || self.owner.to_lowercase().contains(&needle)
            || self.description.to_lowercase().contains(&needle)
    }
}

pub fn parse_dependency(dependency: &str) -> Option<(String, String)> {
    let segments: Vec<&str> = dependency.split('-').collect();
    if segments.len() < 3 {
        return None;
    }
    let version = segments.last()?.to_string();
    if !looks_like_version(&version) {
        return None;
    }
    let full_name = segments[..segments.len() - 1].join("-");
    if full_name.is_empty() {
        return None;
    }
    Some((full_name, version))
}

pub fn is_modloader_pack(full_name: &str) -> bool {
    full_name.contains("BepInExPack")
}

pub fn version_is_newer(candidate: &str, installed: &str) -> bool {
    compare_versions(candidate, installed) == std::cmp::Ordering::Greater
}

pub fn compare_versions(left: &str, right: &str) -> std::cmp::Ordering {
    let left_parts = version_parts(left);
    let right_parts = version_parts(right);
    let length = left_parts.len().max(right_parts.len());
    for index in 0..length {
        let left_value = left_parts.get(index).copied().unwrap_or(0);
        let right_value = right_parts.get(index).copied().unwrap_or(0);
        if left_value != right_value {
            return left_value.cmp(&right_value);
        }
    }
    0u64.cmp(&0u64)
}

fn looks_like_version(value: &str) -> bool {
    let mut segments = value.split('.');
    let Some(major) = segments.next() else {
        return false;
    };
    let Some(minor) = segments.next() else {
        return false;
    };
    let Some(patch) = segments.next() else {
        return false;
    };
    if segments.next().is_some() {
        return false;
    }
    [major, minor, patch]
        .into_iter()
        .all(|segment| !segment.is_empty() && segment.chars().all(|ch| ch.is_ascii_digit()))
}

fn version_parts(version: &str) -> Vec<u64> {
    version
        .split('.')
        .filter_map(|segment| {
            let digits: String = segment.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dependency_identity() {
        let (full_name, version) =
            parse_dependency("cometcake575-Architect-3.32.3").expect("parse");
        assert_eq!(full_name, "cometcake575-Architect");
        assert_eq!(version, "3.32.3");
    }

    #[test]
    fn compares_dotted_versions() {
        assert!(version_is_newer("1.2.3", "1.2.2"));
        assert!(!version_is_newer("1.2.3", "1.2.3"));
        assert!(!version_is_newer("1.2.3", "2.0.0"));
    }

    #[test]
    fn search_matches_owner_and_description() {
        let package = RemotePackage {
            name: "FsmUtil".into(),
            full_name: "silksong_modding-FsmUtil".into(),
            owner: "silksong_modding".into(),
            description: "PlayMaker helpers".into(),
            version: "0.3.17".into(),
            version_full_name: "silksong_modding-FsmUtil-0.3.17".into(),
            downloads: 10,
            download_url: "https://example.invalid".into(),
            icon_url: String::new(),
            dependencies: Vec::new(),
            date_updated: String::new(),
            is_deprecated: false,
        };
        assert!(package.matches_search("playmaker"));
        assert!(package.matches_search("SILKSONG_MODDING"));
        assert!(!package.matches_search("architect"));
    }

    #[test]
    fn builds_icon_url_when_api_icon_missing() {
        let package = RemotePackage::from_api(ApiPackage {
            name: "FsmUtil".into(),
            full_name: "silksong_modding-FsmUtil".into(),
            owner: "silksong_modding".into(),
            date_updated: String::new(),
            is_deprecated: false,
            versions: vec![ApiVersion {
                name: "FsmUtil".into(),
                full_name: "silksong_modding-FsmUtil-0.3.17".into(),
                description: String::new(),
                version_number: "0.3.17".into(),
                downloads: 1,
                download_url: "https://example.invalid".into(),
                icon: String::new(),
                dependencies: Vec::new(),
            }],
        })
        .expect("package");
        assert_eq!(
            package.icon_url,
            "https://gcdn.thunderstore.io/live/repository/icons/silksong_modding-FsmUtil-0.3.17.png"
        );
    }
}
