//! Read-only cloud-drive connections. OAuth credentials never cross the IPC boundary.
use crate::{entry, App, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tauri::{Emitter, Manager, WebviewWindow};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
use url::Url;

const JSON_LIMIT: usize = 2_000_000;
const CHUNK_SIZE: usize = 1024;
const MAX_SECRET_CHUNKS: usize = 64;
const CONNECTIONS_FILE: &str = "cloud-connections.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub provider: String,
    pub label: String,
    pub email: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Secret {
    client_id: String,
    client_secret: String,
    region: String,
    user_id: String,
    #[serde(default)]
    root_namespace: Option<String>,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    generation: String,
    count: usize,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    id: String,
    name: String,
    is_folder: bool,
    mime_type: String,
    size: Option<u64>,
    modified_at: Option<String>,
    web_url: Option<String>,
    can_download: bool,
}
#[derive(Serialize)]
pub struct Page {
    items: Vec<Item>,
    cursor: Option<String>,
}

pub struct CloudState {
    dir: PathBuf,
    http: reqwest::Client,
    connections: Mutex<Vec<Connection>>,
    load_error: Option<String>,
    mutation: Mutex<()>,
    authorization: Mutex<()>,
    refresh_locks: std::sync::Mutex<HashMap<String, Arc<Mutex<()>>>>,
}
impl CloudState {
    pub fn new(dir: PathBuf) -> Result<Self> {
        let loaded = match std::fs::metadata(dir.join(CONNECTIONS_FILE)) {
            Ok(metadata) if metadata.len() > JSON_LIMIT as u64 => Err("The saved cloud connection list is too large. Restore its backup before reconnecting.".to_string()),
            Ok(_) => std::fs::read(dir.join(CONNECTIONS_FILE))
                .map_err(|_| "The saved cloud connection list could not be opened. Restore its backup before reconnecting.".to_string())
                .and_then(|bytes| serde_json::from_slice::<Vec<Connection>>(&bytes)
                    .map_err(|_| "The saved cloud connection list could not be read. Restore its backup before reconnecting.".to_string())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err("The saved cloud connection list could not be opened. Restore its backup before reconnecting.".to_string()),
        };
        let (connections, load_error) = match loaded {
            Ok(connections) => (connections, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Ok(Self {
            dir,
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
                .build()
                .map_err(|_| "Could not initialize the cloud connection client")?,
            connections: Mutex::new(connections),
            load_error,
            mutation: Mutex::new(()),
            authorization: Mutex::new(()),
            refresh_locks: std::sync::Mutex::new(HashMap::new()),
        })
    }
    fn available(&self) -> Result<()> {
        match &self.load_error {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
    async fn connection(&self, id: &str) -> Result<Connection> {
        self.available()?;
        self.connections
            .lock()
            .await
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| "Cloud connection not found. Refresh the connections list.".into())
    }
    fn refresh_lock(&self, id: &str) -> Result<Arc<Mutex<()>>> {
        Ok(self
            .refresh_locks
            .lock()
            .map_err(|_| "Cloud connection is busy")?
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone())
    }
    async fn authorized(&self, connection: &Connection) -> Result<Secret> {
        let mutex = self.refresh_lock(&connection.id)?;
        let _guard = mutex.lock().await;
        self.connection(&connection.id).await?;
        let mut secret = load_secret(&connection.id)?;
        if secret.expires_at <= Utc::now().timestamp() + 90 {
            let refresh = secret
                .refresh_token
                .clone()
                .ok_or("This drive needs sign-in again. Disconnect it and reconnect.")?;
            let mut form = vec![
                ("grant_type", "refresh_token".to_string()),
                ("client_id", secret.client_id.clone()),
                ("refresh_token", refresh),
            ];
            if !secret.client_secret.is_empty() {
                form.push(("client_secret", secret.client_secret.clone()));
            }
            let v = request_json(
                self.http
                    .post(token_endpoint(&connection.provider, &secret.region)?)
                    .form(&form),
            )
            .await?;
            update_token(&mut secret, &v)?;
            save_secret(&connection.id, &secret)?;
        }
        Ok(secret)
    }
    fn persist(&self, connections: &[Connection]) -> Result<()> {
        self.available()?;
        let temp = self.dir.join(format!("cloud-connections-{}.tmp", nonce()));
        let bytes = serde_json::to_vec(connections)
            .map_err(|_| "Could not encode cloud connection list")?;
        std::fs::write(&temp, bytes).map_err(|_| "Could not save cloud connections")?;
        if std::fs::rename(&temp, self.dir.join(CONNECTIONS_FILE)).is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err("Could not update the saved cloud connection list".into());
        }
        Ok(())
    }
}

fn nonce() -> String {
    format!("{:016x}", rand::random::<u64>())
}
fn credential_name(id: &str) -> String {
    format!("cloud:{id}:manifest")
}
fn chunk_name(id: &str, manifest: &Manifest, part: usize) -> String {
    format!("cloud:{id}:{}:{part}", manifest.generation)
}
fn valid_manifest(value: &Manifest) -> bool {
    value.count > 0
        && value.count <= MAX_SECRET_CHUNKS
        && value.generation.len() == 16
        && value.generation.bytes().all(|b| b.is_ascii_hexdigit())
}
fn secret_chunks(secret: &Secret) -> Result<Vec<String>> {
    let encoded = STANDARD
        .encode(serde_json::to_vec(secret).map_err(|_| "Could not encode cloud credentials")?);
    let chunks: Vec<_> = encoded
        .as_bytes()
        .chunks(CHUNK_SIZE)
        .map(|bytes| String::from_utf8(bytes.to_vec()).expect("base64 is ASCII"))
        .collect();
    if chunks.is_empty() || chunks.len() > MAX_SECRET_CHUNKS {
        return Err("Cloud credential response is too large".into());
    }
    Ok(chunks)
}
fn decode_chunks(parts: &[String]) -> Result<Secret> {
    if parts.is_empty()
        || parts.len() > MAX_SECRET_CHUNKS
        || parts.iter().any(|p| p.len() > CHUNK_SIZE)
    {
        return Err("Saved cloud credentials are invalid. Reconnect the drive.".into());
    }
    serde_json::from_slice(
        &STANDARD
            .decode(parts.concat())
            .map_err(|_| "Saved cloud credentials are invalid")?,
    )
    .map_err(|_| "Saved cloud credentials are invalid. Reconnect the drive.".into())
}
fn read_manifest(id: &str) -> Result<Manifest> {
    let m: Manifest = serde_json::from_str(
        &entry(&credential_name(id))?
            .get_password()
            .map_err(|_| "Could not read cloud credentials. Reconnect the drive.")?,
    )
    .map_err(|_| "Saved cloud credentials are invalid")?;
    if !valid_manifest(&m) {
        return Err("Saved cloud credentials are invalid".into());
    }
    Ok(m)
}
fn load_secret(id: &str) -> Result<Secret> {
    let m = read_manifest(id)?;
    let parts: Result<Vec<_>> = (0..m.count)
        .map(|part| {
            entry(&chunk_name(id, &m, part))?
                .get_password()
                .map_err(|_| "Could not read cloud credentials. Reconnect the drive.".into())
        })
        .collect();
    decode_chunks(&parts?)
}
fn delete_parts(id: &str, manifest: &Manifest) {
    for part in 0..manifest.count {
        if let Ok(key) = entry(&chunk_name(id, manifest, part)) {
            let _ = key.delete_credential();
        }
    }
}
fn save_secret(id: &str, secret: &Secret) -> Result<()> {
    // Windows Credential Manager has a small per-credential blob limit. ASCII
    // chunks also support long JWTs without putting any token in a disk file.
    let chunks = secret_chunks(secret)?;
    let previous = read_manifest(id).ok();
    let manifest = Manifest {
        generation: nonce(),
        count: chunks.len(),
    };
    for (part, chunk) in chunks.iter().enumerate() {
        if entry(&chunk_name(id, &manifest, part))?
            .set_password(chunk)
            .is_err()
        {
            delete_parts(id, &manifest);
            return Err("Windows Credential Manager could not save cloud credentials".into());
        }
    }
    if entry(&credential_name(id))?
        .set_password(
            &serde_json::to_string(&manifest)
                .map_err(|_| "Could not encode credential manifest")?,
        )
        .is_err()
    {
        delete_parts(id, &manifest);
        return Err("Windows Credential Manager could not save cloud credentials".into());
    }
    if let Some(previous) = previous {
        delete_parts(id, &previous);
    }
    Ok(())
}
fn delete_secret(id: &str) -> Result<()> {
    if let Ok(manifest) = read_manifest(id) {
        delete_parts(id, &manifest);
    }
    match entry(&credential_name(id))?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("The connection was removed, but Windows could not clear its saved credential manifest.".into()),
    }
}

fn registered(window: &WebviewWindow, app: &App) -> Result<()> {
    if app
        .windows
        .lock()
        .map_err(|_| "Window state is busy")?
        .seeds
        .contains_key(window.label())
    {
        Ok(())
    } else {
        Err("This Yas Mail window has closed".into())
    }
}
fn provider_name(provider: &str) -> Result<&'static str> {
    match provider {
        "google" => Ok("Google Drive"),
        "onedrive" => Ok("OneDrive"),
        "dropbox" => Ok("Dropbox"),
        "zoho" => Ok("Zoho WorkDrive"),
        _ => Err("Choose a supported cloud drive".into()),
    }
}
fn zoho_domains(region: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match region {
        "us" => Ok(("accounts.zoho.com", "www.zohoapis.com", "download.zoho.com")),
        "eu" => Ok(("accounts.zoho.eu", "www.zohoapis.eu", "download.zoho.eu")),
        "in" => Ok(("accounts.zoho.in", "www.zohoapis.in", "download.zoho.in")),
        "au" => Ok((
            "accounts.zoho.com.au",
            "www.zohoapis.com.au",
            "download.zoho.com.au",
        )),
        "jp" => Ok(("accounts.zoho.jp", "www.zohoapis.jp", "download.zoho.jp")),
        "ca" => Ok((
            "accounts.zohocloud.ca",
            "www.zohoapis.ca",
            "download.zohocloud.ca",
        )),
        "cn" => Ok((
            "accounts.zoho.com.cn",
            "www.zohoapis.com.cn",
            "download.zoho.com.cn",
        )),
        "ae" => Ok(("accounts.zoho.ae", "www.zohoapis.ae", "files.zoho.ae")),
        "sa" => Ok(("accounts.zoho.sa", "www.zohoapis.sa", "files.zoho.sa")),
        _ => Err(
            "Choose the Zoho data-center region where your account and client are registered"
                .into(),
        ),
    }
}
fn scopes(provider: &str) -> Result<&'static str> {
    match provider {
        "google" => Ok("https://www.googleapis.com/auth/drive.readonly"),
        "onedrive" => Ok("offline_access User.Read Files.Read"),
        "dropbox" => Ok("account_info.read files.metadata.read files.content.read sharing.read"),
        "zoho" => Ok("WorkDrive.users.READ,WorkDrive.team.READ,WorkDrive.teamfolders.READ,WorkDrive.files.READ,ZohoSearch.securesearch.READ"),
        _ => Err("Unsupported cloud provider".into()),
    }
}
fn token_endpoint(provider: &str, region: &str) -> Result<String> {
    Ok(match provider {
        "google" => "https://oauth2.googleapis.com/token".into(),
        "onedrive" => "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
        "dropbox" => "https://api.dropboxapi.com/oauth2/token".into(),
        "zoho" => format!("https://{}/oauth/v2/token", zoho_domains(region)?.0),
        _ => return Err("Unsupported cloud provider".into()),
    })
}
fn authorization_url(
    provider: &str,
    secret: &Secret,
    redirect: &str,
    state: &str,
    challenge: &str,
) -> Result<Url> {
    let endpoint = match provider {
        "google" => "https://accounts.google.com/o/oauth2/v2/auth".into(),
        "onedrive" => "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
        "dropbox" => "https://www.dropbox.com/oauth2/authorize".into(),
        "zoho" => format!("https://{}/oauth/v2/auth", zoho_domains(&secret.region)?.0),
        _ => return Err("Unsupported cloud provider".into()),
    };
    let mut url = Url::parse(&endpoint).map_err(|_| "Invalid provider authorization endpoint")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", secret.client_id.as_str()),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", scopes(provider)?),
        ("state", state),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
    ]);
    match provider {
        "dropbox" => {
            url.query_pairs_mut()
                .append_pair("token_access_type", "offline");
        }
        "google" | "zoho" => {
            url.query_pairs_mut()
                .append_pair("access_type", "offline")
                .append_pair("prompt", "consent");
        }
        _ => {
            url.query_pairs_mut()
                .append_pair("prompt", "select_account");
        }
    }
    Ok(url)
}
fn constant_time_equal(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0, |difference, (left, right)| difference | (left ^ right))
        == 0
}
fn callback_parameters(request: &str, expected_state: &str) -> Option<HashMap<String, String>> {
    let mut line = request.lines().next()?.split_whitespace();
    if line.next()? != "GET" {
        return None;
    }
    let path = line.next()?;
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    let url = Url::parse(&format!("http://127.0.0.1{path}")).ok()?;
    if url.path() != "/callback" {
        return None;
    }
    let pairs: Vec<_> = url.query_pairs().into_owned().collect();
    if pairs.iter().filter(|(name, _)| name == "state").count() != 1
        || pairs.iter().filter(|(name, _)| name == "code").count() > 1
    {
        return None;
    }
    let pairs: HashMap<_, _> = pairs.into_iter().collect();
    if !constant_time_equal(pairs.get("state")?, expected_state) {
        return None;
    }
    Some(pairs)
}
pub(crate) async fn wait_for_callback(
    listener: TcpListener,
    expected: &str,
) -> Result<HashMap<String, String>> {
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let (mut stream, peer) = listener.accept().await.map_err(|_| "Could not receive provider sign-in")?;
            if !peer.ip().is_loopback() { continue; }
            let request = match tokio::time::timeout(Duration::from_secs(5), read_callback_line(&mut stream)).await {
                Ok(Ok(request)) => request, _ => continue,
            };
            let params = callback_parameters(&request, expected);
            let valid = params.is_some();
            let body = if valid { "Sign-in received. You can return to Yas Mail." } else { "Invalid sign-in callback." };
            let reply = format!("HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{}", if valid {"200 OK"} else {"400 Bad Request"}, body.len(), body);
            let _ = stream.write_all(reply.as_bytes()).await;
            if let Some(params) = params { return Ok(params); }
        }
    }).await.map_err(|_| "Sign-in timed out. Try connecting again.".to_string())?
}
async fn read_callback_line(stream: &mut tokio::net::TcpStream) -> Result<String> {
    let mut line = Vec::new();
    let mut buffer = [0u8; 512];
    loop {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|_| "Could not read the sign-in callback")?;
        if count == 0 {
            return Err("Incomplete sign-in callback".into());
        }
        line.extend_from_slice(&buffer[..count]);
        if let Some(end) = line.windows(2).position(|pair| pair == b"\r\n") {
            if end > 8192 {
                return Err("Sign-in callback line is too long".into());
            }
            return String::from_utf8(line[..end].to_vec())
                .map_err(|_| "Invalid sign-in callback encoding".into());
        }
        if line.len() > 8192 {
            return Err("Sign-in callback line is too long".into());
        }
    }
}
fn update_token(secret: &mut Secret, value: &Value) -> Result<()> {
    secret.access_token = value["access_token"].as_str().filter(|s| !s.is_empty())
        .ok_or("The provider did not return an access token. Check the app registration and reconnect.")?.to_string();
    secret.refresh_token = value["refresh_token"]
        .as_str()
        .map(String::from)
        .or(secret.refresh_token.take());
    let expiry = value["expires_in"]
        .as_i64()
        .or_else(|| value["expires_in"].as_str().and_then(|v| v.parse().ok()))
        .unwrap_or(3600);
    secret.expires_at = Utc::now()
        .timestamp()
        .saturating_add(expiry.clamp(30, 31_536_000));
    Ok(())
}
async fn bounded_bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("The provider file or response exceeds the allowed size".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The cloud download was interrupted. Please retry.")?
    {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err("The provider file or response exceeds the allowed size".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn request_json(request: reqwest::RequestBuilder) -> Result<Value> {
    let response = request
        .send()
        .await
        .map_err(|_| "Could not reach the cloud provider. Check your connection and retry.")?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 => "This drive needs sign-in again. Disconnect it and reconnect.".into(),
            403 => "The provider denied access. Check the app permissions and the file's access policy.".into(),
            429 => "The provider is busy. Please wait and try again.".into(),
            _ => format!("The cloud provider request failed (HTTP {}). Check its app registration or try again.", status.as_u16()),
        });
    }
    serde_json::from_slice(&bounded_bytes(response, JSON_LIMIT).await?)
        .map_err(|_| "The provider returned an unexpected response".into())
}
fn clean_text(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
fn opaque_id(value: &str) -> Result<&str> {
    if value.is_empty() || value.len() > 8192 || value.chars().any(char::is_control) {
        return Err("Invalid cloud file or folder identifier".into());
    }
    Ok(value)
}
fn api_url(base: &str, segments: &[&str]) -> Result<Url> {
    let mut url = Url::parse(base).map_err(|_| "Invalid provider API endpoint")?;
    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| "Invalid provider API path")?;
        path.pop_if_empty();
        for segment in segments {
            path.push(opaque_id(segment)?);
        }
    }
    Ok(url)
}
fn host_matches(host: &str, base: &str) -> bool {
    host == base || host.ends_with(&format!(".{base}"))
}
fn valid_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
        && url.fragment().is_none()
        && url
            .host_str()
            .is_some_and(|host| !host.parse::<std::net::IpAddr>().is_ok())
}
fn web_link(provider: &str, value: Option<&str>) -> Option<String> {
    let value = value?;
    if value.len() > 8192 || value.trim() != value || value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    // Existing document anchors are valid email links; API and download URLs
    // remain fragment-free under valid_https.
    let mut authority = url.clone();
    authority.set_fragment(None);
    if !valid_https(&authority) {
        return None;
    }
    let host = url.host_str()?;
    let allowed: &[&str] = match provider {
        "google" => &["drive.google.com", "docs.google.com"],
        "onedrive" => &[
            "onedrive.live.com",
            "1drv.ms",
            "sharepoint.com",
            "sharepoint-df.com",
        ],
        "dropbox" => &["dropbox.com"],
        "zoho" => &[
            "zoho.com",
            "zoho.eu",
            "zoho.in",
            "zoho.com.au",
            "zoho.jp",
            "zoho.com.cn",
            "zoho.ae",
            "zoho.sa",
            "zohocloud.ca",
            "zohoapis.com",
            "zohoapis.eu",
            "zohoapis.in",
            "zohoapis.com.au",
            "zohoapis.jp",
            "zohoapis.com.cn",
            "zohoapis.ae",
            "zohoapis.sa",
            "zohoapis.ca",
        ],
        _ => return None,
    };
    allowed
        .iter()
        .any(|base| host_matches(host, base))
        .then(|| url.to_string())
}
fn api_request(
    state: &CloudState,
    connection: &Connection,
    secret: &Secret,
    method: reqwest::Method,
    url: Url,
) -> reqwest::RequestBuilder {
    let mut request = state.http.request(method, url);
    if connection.provider == "zoho" {
        request = request
            .header(
                reqwest::header::AUTHORIZATION,
                format!("Zoho-oauthtoken {}", secret.access_token),
            )
            .header(reqwest::header::ACCEPT, "application/vnd.api+json");
    } else {
        request = request.bearer_auth(&secret.access_token);
    }
    if connection.provider == "dropbox" {
        if let Some(namespace) = &secret.root_namespace {
            request = request.header(
                "Dropbox-API-Path-Root",
                json!({".tag":"root","root":namespace}).to_string(),
            );
        }
    }
    request
}
async fn get(state: &CloudState, c: &Connection, s: &Secret, url: Url) -> Result<Value> {
    request_json(api_request(state, c, s, reqwest::Method::GET, url)).await
}
async fn dropbox_post(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
    path: &str,
    body: Value,
) -> Result<Value> {
    let url = api_url(
        "https://api.dropboxapi.com/2/",
        &path.split('/').collect::<Vec<_>>(),
    )?;
    request_json(api_request(state, c, s, reqwest::Method::POST, url).json(&body)).await
}
fn json_rows(value: &Value) -> Vec<&Value> {
    match value {
        Value::Array(rows) => rows.iter().collect(),
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    }
}
fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

#[tauri::command]
pub async fn cloud_connections(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
) -> Result<Vec<Connection>> {
    registered(&window, &app)?;
    state.available()?;
    Ok(state.connections.lock().await.clone())
}
#[tauri::command]
pub async fn cloud_connect(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
    provider: String,
    client_id: Option<String>,
    client_secret: Option<String>,
    region: Option<String>,
) -> Result<Connection> {
    registered(&window, &app)?;
    state.available()?;
    provider_name(&provider)?;
    let _authorization = state.authorization.try_lock().map_err(|_| {
        "Another cloud sign-in is already open. Complete it or wait for it to time out."
    })?;
    let (client_id, client_secret) = if matches!(provider.as_str(), "google" | "onedrive") {
        let client = crate::publisher_oauth::client(&provider)?;
        (client.client_id, client.client_secret)
    } else {
        (
            client_id.unwrap_or_default().trim().to_string(),
            client_secret.unwrap_or_default().trim().to_string(),
        )
    };
    crate::oauth_setup::validate_client_id(&provider, &client_id)?;
    if client_secret.len() > 4096 || client_secret.chars().any(char::is_control) {
        return Err("Enter a valid application client ID and secret, if required".into());
    }
    if provider == "zoho" && client_secret.is_empty() {
        return Err("Zoho desktop OAuth requires the client secret from its API console".into());
    }
    let region = region.unwrap_or_else(|| "us".into());
    if provider == "zoho" {
        zoho_domains(&region)?;
    }
    let port = match provider.as_str() {
        "dropbox" => 53682,
        "zoho" => 53683,
        _ => 0,
    };
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await
        .map_err(|_| "Could not open the local cloud sign-in callback. Close other sign-in attempts and retry.")?;
    let redirect = format!(
        "http://{}:{}/callback",
        if provider == "google" {
            "127.0.0.1"
        } else {
            "localhost"
        },
        listener
            .local_addr()
            .map_err(|_| "Could not read callback port")?
            .port()
    );
    let mut random = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut random);
    let verifier = URL_SAFE_NO_PAD.encode(random);
    rand::thread_rng().fill_bytes(&mut random);
    let csrf = URL_SAFE_NO_PAD.encode(random);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut secret = Secret {
        client_id,
        client_secret,
        region,
        user_id: String::new(),
        root_namespace: None,
        access_token: String::new(),
        refresh_token: None,
        expires_at: 0,
    };
    let auth = authorization_url(&provider, &secret, &redirect, &csrf, &challenge)?;
    webbrowser::open(auth.as_str())
        .map_err(|_| "Could not open the system browser for cloud sign-in")?;
    let params = wait_for_callback(listener, &csrf).await?;
    let code = params
        .get("code")
        .filter(|c| !c.is_empty())
        .ok_or("Cloud sign-in was declined or cancelled")?;
    if provider == "zoho" {
        if let Some(location) = params.get("location") {
            zoho_domains(location)?;
            secret.region = location.clone();
        }
    }
    let mut form = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", secret.client_id.clone()),
        ("code", code.clone()),
        ("redirect_uri", redirect),
        ("code_verifier", verifier),
    ];
    if !secret.client_secret.is_empty() {
        form.push(("client_secret", secret.client_secret.clone()));
    }
    let value = request_json(
        state
            .http
            .post(token_endpoint(&provider, &secret.region)?)
            .form(&form),
    )
    .await?;
    update_token(&mut secret, &value)?;
    let mut connection = Connection {
        id: nonce(),
        provider,
        label: String::new(),
        email: None,
    };
    let (user, label, email, namespace) = profile(&state, &connection, &secret).await?;
    secret.user_id = user;
    secret.root_namespace = namespace;
    connection.label = label;
    connection.email = email;
    registered(&window, &app)?;
    let _mutation = state.mutation.lock().await;
    let existing = state
        .connections
        .lock()
        .await
        .iter()
        .find(|c| {
            c.provider == connection.provider && c.email.is_some() && c.email == connection.email
        })
        .map(|c| c.id.clone());
    if let Some(existing) = &existing {
        connection.id = existing.clone();
    }
    // Reauthentication and refresh use the same per-connection lock. Never
    // hold the connections guard while waiting for a credential refresh.
    let refresh = state.refresh_lock(&connection.id)?;
    let _refresh = refresh.lock().await;
    let mut connections = state.connections.lock().await;
    let mut updated = connections.clone();
    save_secret(&connection.id, &secret)?;
    updated.retain(|c| c.id != connection.id);
    updated.push(connection.clone());
    if let Err(error) = state.persist(&updated) {
        if existing.is_none() {
            let _ = delete_secret(&connection.id);
        }
        return Err(error);
    }
    *connections = updated;
    let _ = window
        .app_handle()
        .emit("yasmail:cloud-connections-changed", json!({}));
    Ok(connection)
}
#[tauri::command]
pub async fn cloud_disconnect(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
    connection: String,
) -> Result<()> {
    registered(&window, &app)?;
    state.available()?;
    let _mutation = state.mutation.lock().await;
    let refresh = state.refresh_lock(&connection)?;
    let _refresh = refresh.lock().await;
    let mut connections = state.connections.lock().await;
    if !connections.iter().any(|c| c.id == connection) {
        return Err("Cloud connection not found".into());
    }
    let updated: Vec<_> = connections
        .iter()
        .filter(|c| c.id != connection)
        .cloned()
        .collect();
    state.persist(&updated)?;
    *connections = updated;
    let deleted = delete_secret(&connection);
    let _ = window
        .app_handle()
        .emit("yasmail:cloud-connections-changed", json!({}));
    deleted
}

// Provider-specific read operations are kept separate from OAuth/persistence.
mod providers;
use providers::{download_request, list, metadata, profile};

#[tauri::command]
pub async fn cloud_list(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
    connection: String,
    folder: Option<String>,
    query: Option<String>,
    cursor: Option<String>,
) -> Result<Page> {
    registered(&window, &app)?;
    if let Some(folder) = &folder {
        opaque_id(folder)?;
    }
    if cursor
        .as_ref()
        .is_some_and(|s| s.len() > 16_384 || s.chars().any(char::is_control))
    {
        return Err("Invalid cloud page cursor".into());
    }
    let query = query.unwrap_or_default().trim().to_string();
    if query.chars().count() > 256 || query.chars().any(char::is_control) {
        return Err("Use a shorter cloud file search".into());
    }
    let c = state.connection(&connection).await?;
    let s = state.authorized(&c).await?;
    let page = list(&state, &c, &s, folder.as_deref(), &query, cursor.as_deref()).await?;
    state.connection(&connection).await?;
    registered(&window, &app)?;
    Ok(page)
}
#[tauri::command]
pub async fn cloud_link(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
    connection: String,
    item: String,
) -> Result<Value> {
    registered(&window, &app)?;
    opaque_id(&item)?;
    let c = state.connection(&connection).await?;
    let s = state.authorized(&c).await?;
    let file = metadata(&state, &c, &s, &item, true).await?;
    if file.is_folder {
        return Err("Choose a file to insert a cloud link".into());
    }
    let link = file.web_url.ok_or("No existing file link is available. Manage access at the provider, or attach a copy. Yas Mail does not create public links.")?;
    registered(&window, &app)?;
    state.connection(&connection).await?;
    Ok(json!({"webUrl":link}))
}
#[tauri::command]
pub async fn cloud_download(
    window: WebviewWindow,
    app: tauri::State<'_, Arc<App>>,
    state: tauri::State<'_, Arc<CloudState>>,
    connection: String,
    item: String,
    mail_account: String,
) -> Result<Value> {
    crate::windows::check_owner(&app, window.label(), &mail_account)?;
    let mail = app.account(&mail_account).await?;
    let limit = if mail.provider == "outlook" {
        3_000_000usize
    } else {
        9_000_000usize
    };
    opaque_id(&item)?;
    let c = state.connection(&connection).await?;
    let s = state.authorized(&c).await?;
    let file = metadata(&state, &c, &s, &item, false).await?;
    if file.is_folder || !file.can_download {
        return Err(
            "This online document cannot be attached as a file. Insert its existing link instead."
                .into(),
        );
    }
    if file.size.is_some_and(|size| size > limit as u64) {
        return Err(format!("This file exceeds this email account's {} MB attachment limit. Insert its link instead.", limit / 1_000_000));
    }
    let request = download_request(&state, &c, &s, &item)?;
    let mut response = request
        .send()
        .await
        .map_err(|_| "Could not download the cloud file")?;
    for _ in 0..5 {
        if !response.status().is_redirection() {
            break;
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or("The provider did not provide a download location")?;
        let url = Url::parse(location).map_err(|_| "Invalid provider download location")?;
        if !providers::allowed_download_redirect(&c.provider, &url) {
            return Err("The provider returned an unsupported download host".into());
        }
        // Pre-authorized CDN URLs must not receive the OAuth bearer credential.
        response = state
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| "Could not download the cloud file")?;
    }
    if !response.status().is_success() {
        return Err(format!(
            "Cloud download failed (HTTP {}). Check file access or reconnect the drive.",
            response.status().as_u16()
        ));
    }
    let bytes = bounded_bytes(response, limit).await?;
    crate::windows::check_owner(&app, window.label(), &mail_account)?;
    app.account(&mail_account).await?;
    state.connection(&connection).await?;
    Ok(json!({"name":file.name,"data":STANDARD.encode(&bytes),"size":bytes.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn secret() -> Secret {
        Secret {
            client_id: "client".into(),
            client_secret: "not-a-real-secret".into(),
            region: "us".into(),
            user_id: "user".into(),
            root_namespace: None,
            access_token: "x".repeat(7000),
            refresh_token: Some("r".repeat(5000)),
            expires_at: 123,
        }
    }
    #[test]
    fn long_credentials_round_trip_without_large_windows_blobs() {
        let original = secret();
        let chunks = secret_chunks(&original).unwrap();
        assert!(chunks.len() > 10);
        assert!(chunks.iter().all(|c| c.len() <= 1024));
        let restored = decode_chunks(&chunks).unwrap();
        assert_eq!(restored.access_token, original.access_token);
        assert_eq!(restored.refresh_token, original.refresh_token);
    }
    #[test]
    fn credential_manifest_rejects_pathlike_or_excessive_parts() {
        assert!(!valid_manifest(&Manifest {
            generation: "../../metadata!!".into(),
            count: 1
        }));
        assert!(!valid_manifest(&Manifest {
            generation: "0123456789abcdef".into(),
            count: 65
        }));
    }
    #[test]
    fn oauth_callback_requires_get_exact_path_and_one_matching_state() {
        let good = "GET /callback?code=one&state=secret HTTP/1.1\r\n";
        assert_eq!(callback_parameters(good, "secret").unwrap()["code"], "one");
        for request in [
            "POST /callback?code=one&state=secret HTTP/1.1",
            "GET /wrong?code=one&state=secret HTTP/1.1",
            "GET /callback?code=one&state=wrong HTTP/1.1",
            "GET /callback?code=one&state=secret&state=secret HTTP/1.1",
            "GET //evil.invalid/callback?state=secret HTTP/1.1",
        ] {
            assert!(callback_parameters(request, "secret").is_none());
        }
    }
    #[test]
    fn oauth_requests_only_read_scopes_and_s256() {
        for provider in ["google", "onedrive", "dropbox", "zoho"] {
            let url = authorization_url(
                provider,
                &secret(),
                "http://localhost:123/callback",
                "state",
                "challenge",
            )
            .unwrap();
            let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
            assert_eq!(q["code_challenge_method"], "S256");
            assert!(!q["scope"].contains("Write"));
            assert!(!q["scope"].contains("CREATE"));
            assert_eq!(q["state"], "state");
        }
    }
    #[test]
    fn token_refresh_preserves_or_rotates_refresh_token() {
        let mut s = secret();
        let previous = s.refresh_token.clone();
        update_token(&mut s, &json!({"access_token":"new","expires_in":3600})).unwrap();
        assert_eq!(s.refresh_token, previous);
        update_token(
            &mut s,
            &json!({"access_token":"next","refresh_token":"rotated","expires_in":"3600"}),
        )
        .unwrap();
        assert_eq!(s.refresh_token.as_deref(), Some("rotated"));
    }
    #[test]
    fn provider_ids_are_single_escaped_path_segments() {
        let url = api_url(
            "https://graph.microsoft.com/v1.0/",
            &["me", "drive", "items", "x/y?z#frag"],
        )
        .unwrap();
        assert!(url.as_str().contains("x%2Fy%3Fz%23frag"));
        assert!(url.query().is_none());
        assert!(opaque_id("bad\nvalue").is_err());
    }
    #[test]
    fn inserted_links_reject_lookalikes_credentials_and_external_hosts() {
        for url in [
            "http://drive.google.com/file/x",
            "https://drive.google.com.evil.invalid/x",
            "https://user:password@drive.google.com/file/x",
            "https://drive.google.com:444/x",
            "https://127.0.0.1/x",
        ] {
            assert!(web_link("google", Some(url)).is_none());
        }
        assert!(web_link(
            "google",
            Some("https://docs.google.com/document/d/example/edit")
        )
        .is_some());
        assert!(web_link(
            "onedrive",
            Some("https://tenant.sharepoint.com/sites/team/file")
        )
        .is_some());
        assert!(web_link(
            "google",
            Some("https://docs.google.com/document/d/example/edit#heading=chapter")
        )
        .is_some());
        assert!(web_link(
            "google",
            Some(" https://docs.google.com/document/d/example/edit")
        )
        .is_none());
        assert!(web_link(
            "google",
            Some("https://docs.google.com/document/d/exam\nple/edit")
        )
        .is_none());
    }

    #[test]
    fn corrupt_optional_cloud_metadata_does_not_disable_app_startup_or_get_overwritten() {
        let dir = std::env::temp_dir().join(format!("yas-mail-cloud-metadata-{}", nonce()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join(CONNECTIONS_FILE);
        let original = b"not valid JSON; preserve this existing file";
        std::fs::write(&path, original).unwrap();
        let state = CloudState::new(dir.clone())
            .expect("optional cloud metadata must not prevent app startup");
        assert!(state.available().is_err());
        assert!(state.persist(&[]).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn public_connection_metadata_never_contains_client_credentials_or_tokens() {
        let c = Connection {
            id: "opaque-id".into(),
            provider: "google".into(),
            label: "Work drive".into(),
            email: Some("test@example.invalid".into()),
        };
        let public = serde_json::to_value(c).unwrap();
        assert_eq!(public.as_object().unwrap().len(), 4);
        for name in ["clientId", "clientSecret", "accessToken", "refreshToken"] {
            assert!(public.get(name).is_none());
        }
    }

    #[tokio::test]
    async fn oauth_callback_reads_a_split_tcp_request_line() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let sender = tokio::spawn(async move {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(b"GET /callback?co").await.unwrap();
            tokio::task::yield_now().await;
            stream
                .write_all(b"de=example&state=expected HTTP/1.1\r\nHost: localhost\r\n\r\n")
                .await
                .unwrap();
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let line = read_callback_line(&mut stream).await.unwrap();
        assert_eq!(
            callback_parameters(&line, "expected").unwrap()["code"],
            "example"
        );
        sender.await.unwrap();
    }

    #[tokio::test]
    async fn cloud_download_limit_applies_when_server_omits_content_length() {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request).await;
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n").await.unwrap();
        });
        // This HTTP client is confined to an isolated loopback test fixture;
        // the production CloudState client permits HTTPS only.
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap();
        assert!(bounded_bytes(response, 5).await.is_err());
        server.await.unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Opt-in Windows Credential Manager smoke; writes and removes only a unique synthetic test ID"]
    fn windows_credential_manager_long_cloud_secret_round_trip() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                // The guard owns this unique synthetic ID only. It also runs
                // during unwinding, so a failed assertion leaves no test keys.
                let _ = delete_secret(&self.0);
            }
        }
        let id = format!("yas-mail-cloud-test-{}-{}", std::process::id(), nonce());
        let _cleanup = Cleanup(id.clone());
        let original = secret();
        save_secret(&id, &original)
            .expect("Windows could not save the synthetic cloud credentials");
        let manifest = read_manifest(&id).expect("Synthetic credential manifest missing");
        assert!(
            manifest.count > 10,
            "The long synthetic tokens must span multiple safe-size credentials"
        );
        let restored =
            load_secret(&id).expect("Windows could not load the synthetic cloud credentials");
        assert!(
            restored.access_token == original.access_token,
            "Synthetic access-token round trip mismatch"
        );
        assert!(
            restored.refresh_token == original.refresh_token,
            "Synthetic refresh-token round trip mismatch"
        );
        assert!(
            restored.client_secret == original.client_secret,
            "Synthetic client-metadata round trip mismatch"
        );
        delete_secret(&id).expect("Windows could not remove the synthetic cloud credentials");
        assert!(
            matches!(
                entry(&credential_name(&id)).unwrap().get_password(),
                Err(keyring::Error::NoEntry)
            ),
            "Synthetic manifest was not removed"
        );
        for part in 0..manifest.count {
            assert!(
                matches!(
                    entry(&chunk_name(&id, &manifest, part))
                        .unwrap()
                        .get_password(),
                    Err(keyring::Error::NoEntry)
                ),
                "A synthetic credential chunk was not removed"
            );
        }
    }
}
