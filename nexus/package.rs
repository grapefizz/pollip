use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMod {
    pub mod_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub picture_url: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub uploaded_by: String,
    #[serde(default)]
    pub endorsement_count: u64,
    #[serde(default)]
    pub domain_name: String,
    #[serde(default)]
    pub available: bool,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiValidate {
    pub user_id: u64,
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
    #[serde(default)]
    pub is_supporter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiFileList {
    #[serde(default)]
    pub files: Vec<ApiFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiFile {
    pub file_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub category_name: String,
    #[serde(default)]
    pub is_primary: bool,
    #[serde(default)]
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDownloadLink {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub short_name: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteMod {
    pub mod_id: u64,
    pub name: String,
    pub author: String,
    pub version: String,
    pub endorsement_count: u64,
    pub description: String,
    pub summary: String,
    pub picture_url: String,
    pub domain_name: String,
}

impl RemoteMod {
    pub fn from_api(mod_info: ApiMod) -> Option<Self> {
        if !mod_info.available && mod_info.name.is_empty() {
            return None;
        }
        let name = if mod_info.name.is_empty() {
            format!("mod {}", mod_info.mod_id)
        } else {
            mod_info.name
        };
        let author = if !mod_info.author.trim().is_empty() {
            mod_info.author
        } else if !mod_info.uploaded_by.trim().is_empty() {
            mod_info.uploaded_by
        } else {
            "unknown".to_string()
        };
        let description = if !mod_info.summary.trim().is_empty() {
            mod_info.summary.clone()
        } else {
            strip_bbcode(&mod_info.description)
        };
        Some(Self {
            mod_id: mod_info.mod_id,
            name,
            author,
            version: mod_info.version,
            endorsement_count: mod_info.endorsement_count,
            description,
            summary: mod_info.summary,
            picture_url: mod_info.picture_url,
            domain_name: if mod_info.domain_name.is_empty() {
                super::domain::GAME_DOMAIN.to_string()
            } else {
                mod_info.domain_name
            },
        })
    }

    pub fn matches_search(&self, query: &str) -> bool {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        self.name.to_lowercase().contains(&needle)
            || self.author.to_lowercase().contains(&needle)
            || self.description.to_lowercase().contains(&needle)
            || self.summary.to_lowercase().contains(&needle)
            || self.mod_id.to_string().contains(&needle)
    }

    pub fn page_url(&self) -> String {
        format!(
            "https://www.nexusmods.com/{}/mods/{}",
            self.domain_name, self.mod_id
        )
    }
}

fn strip_bbcode(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '[' {
            while let Some(inner) = chars.next() {
                if inner == ']' {
                    break;
                }
            }
            continue;
        }
        output.push(ch);
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}
