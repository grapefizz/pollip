use super::domain::{
    user_agent, API_BASE, APP_NAME, APP_VERSION, CATALOG_PAGE_SIZE, GRAPHQL_URL,
};
use super::package::{ApiDownloadLink, ApiFileList, ApiMod, ApiValidate};
use serde::Deserialize;
use serde_json::json;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct ValidateResult {
    pub username: String,
    pub is_premium: bool,
    pub is_supporter: bool,
}

#[derive(Debug)]
pub enum ClientError {
    Network(String),
    Unauthorized,
    RateLimited,
    Forbidden(String),
    Decode(String),
    Http { status: u32, detail: String },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(detail) => write!(
                formatter,
                "could not reach nexus mods (check your network): {detail}"
            ),
            Self::Unauthorized => write!(formatter, "nexus api key is invalid or revoked"),
            Self::RateLimited => write!(
                formatter,
                "rate limited by nexus mods — try again later (daily/hourly quota)"
            ),
            Self::Forbidden(detail) => write!(formatter, "nexus request forbidden: {detail}"),
            Self::Decode(detail) => write!(formatter, "could not parse nexus response: {detail}"),
            Self::Http { status, detail } => {
                write!(formatter, "nexus http {status}: {detail}")
            }
        }
    }
}

impl std::error::Error for ClientError {}

pub fn validate_api_key(api_key: &str) -> Result<ValidateResult, ClientError> {
    let body = request_json("GET", &format!("{API_BASE}/users/validate.json"), api_key, None)?;
    let info: ApiValidate =
        serde_json::from_str(&body).map_err(|error| ClientError::Decode(error.to_string()))?;
    Ok(ValidateResult {
        username: info.name,
        is_premium: info.is_premium,
        is_supporter: info.is_supporter,
    })
}

pub fn fetch_mod_list_pages(api_key: &str, game_domain: &str) -> Result<Vec<ApiMod>, ClientError> {
    let mut combined = Vec::new();
    let mut seen = Vec::new();
    let mut offset = 0u32;
    let mut expected_total: Option<u64> = None;

    loop {
        let page = fetch_mods_graphql_page(api_key, game_domain, offset, CATALOG_PAGE_SIZE)?;
        if expected_total.is_none() {
            expected_total = Some(page.total_count);
        }
        if page.nodes.is_empty() {
            break;
        }

        let page_len = page.nodes.len() as u32;
        for node in page.nodes {
            if seen.contains(&node.mod_id) {
                continue;
            }
            seen.push(node.mod_id);
            combined.push(node.into_api_mod(game_domain));
        }

        offset = offset.saturating_add(page_len);
        let total = expected_total.unwrap_or(0);
        if u64::from(offset) >= total || page_len == 0 {
            break;
        }

        if offset > 50_000 {
            break;
        }
    }

    Ok(combined)
}

fn fetch_mods_graphql_page(
    api_key: &str,
    game_domain: &str,
    offset: u32,
    count: u32,
) -> Result<GraphqlModsPage, ClientError> {
    const QUERY: &str = r#"
query($filter: ModsFilter, $count: Int, $offset: Int, $sort: [ModsSort!]) {
  mods(filter: $filter, count: $count, offset: $offset, sort: $sort) {
    totalCount
    nodes {
      modId
      name
      version
      summary
      author
      pictureUrl
      endorsements
      status
      game { domainName }
      uploader { name }
    }
  }
}
"#;

    let payload = json!({
        "query": QUERY,
        "variables": {
            "filter": {
                "gameDomainName": [{ "value": game_domain, "op": "EQUALS" }]
            },
            "count": count,
            "offset": offset,
            "sort": [{ "endorsements": { "direction": "DESC" } }]
        }
    });
    let body = request_json(
        "POST",
        GRAPHQL_URL,
        api_key,
        Some(&payload.to_string()),
    )?;
    let envelope: GraphqlEnvelope = serde_json::from_str(&body)
        .map_err(|error| ClientError::Decode(error.to_string()))?;
    if let Some(errors) = envelope.errors.filter(|entries| !entries.is_empty()) {
        let detail = errors
            .into_iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ClientError::Decode(format!("graphql error: {detail}")));
    }
    envelope
        .data
        .map(|data| data.mods)
        .ok_or_else(|| ClientError::Decode("graphql response missing mods data".to_string()))
}

#[derive(Debug, Deserialize)]
struct GraphqlEnvelope {
    data: Option<GraphqlData>,
    #[serde(default)]
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlData {
    mods: GraphqlModsPage,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlModsPage {
    #[serde(rename = "totalCount", default)]
    total_count: u64,
    #[serde(default)]
    nodes: Vec<GraphqlModNode>,
}

#[derive(Debug, Deserialize)]
struct GraphqlModNode {
    #[serde(rename = "modId")]
    mod_id: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    author: String,
    #[serde(rename = "pictureUrl", default)]
    picture_url: String,
    #[serde(default)]
    endorsements: u64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    game: Option<GraphqlGame>,
    #[serde(default)]
    uploader: Option<GraphqlUploader>,
}

#[derive(Debug, Deserialize)]
struct GraphqlGame {
    #[serde(rename = "domainName", default)]
    domain_name: String,
}

#[derive(Debug, Deserialize)]
struct GraphqlUploader {
    #[serde(default)]
    name: String,
}

impl GraphqlModNode {
    fn into_api_mod(self, fallback_domain: &str) -> ApiMod {
        let domain_name = self
            .game
            .map(|game| game.domain_name)
            .filter(|domain| !domain.is_empty())
            .unwrap_or_else(|| fallback_domain.to_string());
        let uploaded_by = self
            .uploader
            .map(|uploader| uploader.name)
            .unwrap_or_default();
        let available = self.status.is_empty()
            || self.status.eq_ignore_ascii_case("published")
            || self.status.eq_ignore_ascii_case("under_moderation");
        ApiMod {
            mod_id: self.mod_id,
            name: self.name,
            summary: self.summary,
            description: String::new(),
            picture_url: self.picture_url,
            version: self.version,
            author: self.author,
            uploaded_by,
            endorsement_count: self.endorsements,
            domain_name,
            available,
            status: self.status,
        }
    }
}

pub fn fetch_mod_files(
    api_key: &str,
    game_domain: &str,
    mod_id: u64,
) -> Result<ApiFileList, ClientError> {
    let url = format!("{API_BASE}/games/{game_domain}/mods/{mod_id}/files.json");
    let body = request_json("GET", &url, api_key, None)?;
    serde_json::from_str(&body).map_err(|error| ClientError::Decode(error.to_string()))
}

pub fn fetch_download_links_premium(
    api_key: &str,
    game_domain: &str,
    mod_id: u64,
    file_id: u64,
) -> Result<Vec<ApiDownloadLink>, ClientError> {
    let url = format!(
        "{API_BASE}/games/{game_domain}/mods/{mod_id}/files/{file_id}/download_link.json"
    );
    let body = request_json("GET", &url, api_key, None)?;
    serde_json::from_str(&body).map_err(|error| ClientError::Decode(error.to_string()))
}

pub fn fetch_download_links_nxm(
    api_key: &str,
    game_domain: &str,
    mod_id: u64,
    file_id: u64,
    key: &str,
    expires: &str,
) -> Result<Vec<ApiDownloadLink>, ClientError> {
    let url = format!(
        "{API_BASE}/games/{game_domain}/mods/{mod_id}/files/{file_id}/download_link.json?key={key}&expires={expires}"
    );
    let body = request_json("GET", &url, api_key, None)?;
    serde_json::from_str(&body).map_err(|error| ClientError::Decode(error.to_string()))
}

pub fn download_file_to(api_key: &str, url: &str, destination: &std::path::Path) -> Result<(), ClientError> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ClientError::Network(format!("could not create download directory: {error}"))
        })?;
    }
    let temporary = destination.with_extension("partial");
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "300",
            "--header",
            &format!("apikey: {api_key}"),
            "--header",
            &format!("User-Agent: {}", user_agent()),
            "--header",
            &format!("Application-Name: {APP_NAME}"),
            "--header",
            &format!("Application-Version: {APP_VERSION}"),
            "--output",
        ])
        .arg(&temporary)
        .arg(url)
        .output()
        .map_err(|error| ClientError::Network(format!("failed to spawn curl: {error}")))?;

    if !output.status.success() {
        let _ = std::fs::remove_file(&temporary);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ClientError::Network(if stderr.trim().is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr.trim().to_string()
        }));
    }

    std::fs::rename(&temporary, destination)
        .map_err(|error| ClientError::Network(format!("could not finalize download: {error}")))?;
    Ok(())
}

fn request_json(
    method: &str,
    url: &str,
    api_key: &str,
    body: Option<&str>,
) -> Result<String, ClientError> {
    let mut command = Command::new("curl");
    command.args([
        "--silent",
        "--show-error",
        "--location",
        "--max-time",
        "60",
        "--write-out",
        "\n%{http_code}",
        "--request",
        method,
        "--header",
        &format!("apikey: {api_key}"),
        "--header",
        &format!("User-Agent: {}", user_agent()),
        "--header",
        &format!("Application-Name: {APP_NAME}"),
        "--header",
        &format!("Application-Version: {APP_VERSION}"),
        "--header",
        "Accept: application/json",
    ]);
    if let Some(payload) = body {
        command.args(["--header", "Content-Type: application/json", "--data", payload]);
    }
    command.arg(url);

    let output = command
        .output()
        .map_err(|error| ClientError::Network(format!("failed to spawn curl: {error}")))?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ClientError::Network(if stderr.trim().is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr.trim().to_string()
        }));
    }

    let raw = String::from_utf8(output.stdout)
        .map_err(|error| ClientError::Decode(format!("response was not utf-8: {error}")))?;
    let (response_body, status) = split_status(&raw)?;
    match status {
        200 | 201 => Ok(response_body),
        401 => Err(ClientError::Unauthorized),
        403 => Err(ClientError::Forbidden(response_body)),
        429 => Err(ClientError::RateLimited),
        other => Err(ClientError::Http {
            status: other,
            detail: response_body,
        }),
    }
}

fn split_status(raw: &str) -> Result<(String, u32), ClientError> {
    let trimmed = raw.trim_end();
    let Some((body, status_text)) = trimmed.rsplit_once('\n') else {
        return Err(ClientError::Decode(
            "curl response missing http status trailer".to_string(),
        ));
    };
    let status: u32 = status_text.trim().parse().map_err(|_| {
        ClientError::Decode(format!("could not parse http status '{status_text}'"))
    })?;
    Ok((body.to_string(), status))
}
