#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use chrono::Utc;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use tauri::Manager;
use tokio::{net::TcpListener, sync::Mutex};
mod ai;
mod cloud;
mod mail_credentials;
mod mail_signin;
mod manual_mail;
mod oauth_setup;
mod publisher_oauth;
mod windows;
type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
#[derive(Clone, Serialize, Deserialize)]
struct Account {
    id: String,
    provider: String,
    email: String,
    client_id: String,
    client_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manual: Option<manual_mail::ManualSettings>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Token {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
}
struct App {
    accounts: Mutex<Vec<Account>>,
    tokens: Mutex<HashMap<String, Token>>,
    style_guard: Mutex<()>,
    mail_sign_ins: Arc<mail_signin::Registry>,
    windows: std::sync::Mutex<windows::WindowRegistry>,
    window_creation: Mutex<()>,
    http: reqwest::Client,
    dir: std::path::PathBuf,
}
fn entry(id: &str) -> Result<keyring::Entry> {
    // Retain the legacy credential namespace so a display-name upgrade keeps
    // existing account tokens, client metadata, and local writing examples.
    keyring::Entry::new("GoodMail", id).map_err(err)
}
fn google(a: &Account) -> bool {
    a.provider == "gmail"
}
fn public_account(a: &Account) -> Value {
    let (read, actions, drafts, extra, attachments, folders, label) =
        match a.manual.as_ref().map(|s| s.protocol.as_str()) {
            Some("imap") => (
                true,
                true,
                true,
                false,
                true,
                vec![
                    "inbox", "starred", "sent", "drafts", "archive", "spam", "trash",
                ],
                "IMAP / SMTP",
            ),
            Some("pop3") => (
                true,
                false,
                false,
                false,
                true,
                vec!["inbox"],
                "POP3 / SMTP",
            ),
            Some("smtp") => (
                false,
                false,
                false,
                false,
                false,
                vec![],
                "SMTP · send only",
            ),
            _ => (
                true,
                true,
                true,
                true,
                true,
                vec![
                    "inbox", "starred", "sent", "drafts", "archive", "spam", "trash",
                ],
                if google(a) {
                    "Gmail"
                } else {
                    "Outlook / Microsoft 365"
                },
            ),
        };
    json!({"id":a.id,"provider":a.provider,"email":a.email,"manual":a.manual,"displayName":a.manual.as_ref().map(|s|s.display_name.as_str()).unwrap_or(""),"providerLabel":label,"capabilities":{"mailRead":read,"mailSearch":read && a.provider != "pop3","mailSend":true,"mailActions":actions,"drafts":drafts,"contacts":extra,"calendar":extra,"rules":extra,"attachments":attachments,"supportedFolders":folders}})
}
fn require_groupware(a: &Account, feature: &str) -> Result<()> {
    if a.manual.is_some() {
        return Err(format!("{feature} sync is not provided by this mail protocol. Use a Google or Microsoft 365 account for this feature."));
    }
    Ok(())
}
fn manual_credentials(a: &Account) -> Result<manual_mail::Credentials> {
    mail_credentials::load(&format!("{}:manual", a.id))
}
fn token_url(a: &Account) -> &'static str {
    if google(a) {
        "https://oauth2.googleapis.com/token"
    } else {
        "https://login.microsoftonline.com/common/oauth2/v2.0/token"
    }
}
async fn response(r: reqwest::Response) -> Result<Value> {
    let status = r.status();
    let text = r.text().await.map_err(err)?;
    if !status.is_success() {
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let message = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| v.get("error_description").and_then(Value::as_str))
            .unwrap_or("Provider request failed. Check permissions and account setup.");
        return Err(format!("{}: {}", status, message));
    }
    if text.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_str(&text).map_err(err)
    }
}
impl App {
    async fn account(&self, id: &str) -> Result<Account> {
        self.accounts
            .lock()
            .await
            .iter()
            .find(|a| a.id == id)
            .cloned()
            .ok_or("Account not found".into())
    }
    async fn token(&self, a: &Account) -> Result<String> {
        if a.manual.is_some() || !["gmail", "outlook"].contains(&a.provider.as_str()) {
            return Err("This account does not use Google or Microsoft OAuth".into());
        }
        let mut tokens = self.tokens.lock().await;
        let mut t = if let Some(t) = tokens.get(&a.id) {
            t.clone()
        } else {
            mail_credentials::load(&a.id)?
        };
        if t.expires_at < Utc::now().timestamp() + 60 {
            let refresh = t.refresh_token.clone().ok_or("Sign in again".to_string())?;
            let mut form = vec![
                ("grant_type", "refresh_token"),
                ("client_id", &a.client_id),
                ("refresh_token", &refresh),
            ];
            if !a.client_secret.is_empty() {
                form.push(("client_secret", &a.client_secret));
            }
            let v = response(
                self.http
                    .post(token_url(a))
                    .form(&form)
                    .send()
                    .await
                    .map_err(err)?,
            )
            .await?;
            t.access_token = v["access_token"]
                .as_str()
                .ok_or("Missing access token")?
                .into();
            t.refresh_token = v["refresh_token"]
                .as_str()
                .map(String::from)
                .or(t.refresh_token);
            t.expires_at = Utc::now().timestamp() + v["expires_in"].as_i64().unwrap_or(3600);
            mail_credentials::save(&a.id, &t)?;
        }
        let access = t.access_token.clone();
        tokens.insert(a.id.clone(), t);
        Ok(access)
    }
    async fn api(&self, a: &Account, method: &str, url: &str, body: Value) -> Result<Value> {
        let mut r = self
            .http
            .request(method.parse::<reqwest::Method>().map_err(err)?, url)
            .bearer_auth(self.token(a).await?);
        if !body.is_null() {
            r = r.json(&body);
        }
        response(r.send().await.map_err(err)?).await
    }
    async fn persist(&self) -> Result<()> {
        let accounts = self.accounts.lock().await;
        // Client IDs are public. Google desktop client metadata is kept with tokens in Credential Manager.
        let public: Vec<_> = accounts
            .iter()
            .map(|a| {
                let mut a = a.clone();
                a.client_secret.clear();
                a
            })
            .collect();
        std::fs::write(
            self.dir.join("accounts.json"),
            serde_json::to_vec(&public).map_err(err)?,
        )
        .map_err(err)
    }
}
#[tauri::command]
async fn accounts(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
) -> Result<Value> {
    let accounts = state.accounts.lock().await.clone();
    let registry = state.windows.lock().map_err(err)?;
    if !registry.seeds.contains_key(window.label()) {
        return Err("Window is no longer registered".into());
    }
    Ok(json!(accounts
        .iter()
        .filter(|a| registry
            .owners
            .get(&a.id)
            .is_some_and(|owner| owner == window.label()))
        .map(public_account)
        .collect::<Vec<_>>()))
}
#[tauri::command]
async fn connect(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    provider: String,
) -> Result<Value> {
    if !["gmail", "outlook"].contains(&provider.as_str()) {
        return Err("Choose Gmail or Outlook to sign in".into());
    }
    let client = publisher_oauth::client(&provider)?;
    if !state
        .windows
        .lock()
        .map_err(err)?
        .seeds
        .contains_key(window.label())
    {
        return Err("Window is no longer registered".into());
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.map_err(err)?;
    let redirect = format!(
        "http://{}:{}/callback",
        if provider == "gmail" {
            "127.0.0.1"
        } else {
            "localhost"
        },
        listener.local_addr().map_err(err)?.port()
    );
    let mut random = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut random);
    let verifier = B64.encode(random);
    rand::thread_rng().fill_bytes(&mut random);
    let csrf = B64.encode(random);
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let mut a = Account {
        id: String::new(),
        provider,
        email: String::new(),
        client_id: client.client_id,
        client_secret: client.client_secret,
        manual: None,
    };
    let scope = if google(&a) {
        "https://www.googleapis.com/auth/gmail.modify https://www.googleapis.com/auth/gmail.settings.basic https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/contacts"
    } else {
        "offline_access User.Read Mail.ReadWrite Mail.Send MailboxSettings.ReadWrite Calendars.ReadWrite Contacts.ReadWrite"
    };
    let mut u = url::Url::parse(if google(&a) {
        "https://accounts.google.com/o/oauth2/v2/auth"
    } else {
        "https://login.microsoftonline.com/common/oauth2/v2.0/authorize"
    })
    .map_err(err)?;
    u.query_pairs_mut().extend_pairs([
        ("client_id", a.client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("response_type", "code"),
        ("scope", scope),
        ("state", csrf.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("prompt", "consent"),
    ]);
    if google(&a) {
        u.query_pairs_mut().append_pair("access_type", "offline");
    }
    let pending = state.mail_sign_ins.begin(window.label())?;
    webbrowser::open(u.as_str()).map_err(err)?;
    let callback = tokio::select! {
        biased;
        _ = pending.cancelled() => return Err("Sign-in cancelled.".into()),
        callback = cloud::wait_for_callback(listener, &csrf) => callback,
    };
    if !pending.accept_callback() {
        return Err("Sign-in cancelled.".into());
    }
    drop(pending);
    let params = callback?;
    let code = params
        .get("code")
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or("Sign-in was cancelled or declined. Please try again.")?;
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("client_id", &a.client_id),
        ("code", &code),
        ("redirect_uri", &redirect),
        ("code_verifier", &verifier),
    ];
    if !a.client_secret.is_empty() {
        form.push(("client_secret", &a.client_secret));
    }
    let v = response(
        state
            .http
            .post(token_url(&a))
            .form(&form)
            .send()
            .await
            .map_err(err)?,
    )
    .await?;
    let token = Token {
        access_token: v["access_token"].as_str().ok_or("Missing token")?.into(),
        refresh_token: v["refresh_token"].as_str().map(String::from),
        expires_at: Utc::now().timestamp() + v["expires_in"].as_i64().unwrap_or(3600),
    };
    let profile = response(
        state
            .http
            .get(if google(&a) {
                "https://gmail.googleapis.com/gmail/v1/users/me/profile"
            } else {
                "https://graph.microsoft.com/v1.0/me"
            })
            .bearer_auth(&token.access_token)
            .send()
            .await
            .map_err(err)?,
    )
    .await?;
    a.email = if google(&a) {
        profile["emailAddress"].as_str()
    } else {
        profile["mail"]
            .as_str()
            .or(profile["userPrincipalName"].as_str())
    }
    .ok_or("Missing account email")?
    .into();
    a.id = format!("{}:{}", a.provider, a.email);
    let _creation = state.window_creation.lock().await;
    if !state
        .windows
        .lock()
        .map_err(err)?
        .seeds
        .contains_key(window.label())
    {
        return Err("The sign-in window has closed".into());
    }
    let mut tokens = state.tokens.lock().await;
    mail_credentials::save(&a.id, &token)?;
    mail_credentials::save(&format!("{}:metadata", a.id), &a)?;
    tokens.insert(a.id.clone(), token);
    drop(tokens);
    {
        let mut accounts = state.accounts.lock().await;
        accounts.retain(|x| x.id != a.id);
        accounts.push(a.clone());
    }
    state
        .windows
        .lock()
        .map_err(err)?
        .owners
        .insert(a.id.clone(), window.label().into());
    state.persist().await?;
    windows::changed(window.app_handle());
    Ok(public_account(&a))
}
#[tauri::command]
async fn manual_test(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    settings: manual_mail::ManualSettings,
    credentials: manual_mail::Credentials,
) -> Result<Value> {
    if !state
        .windows
        .lock()
        .map_err(err)?
        .seeds
        .contains_key(window.label())
    {
        return Err("Window is no longer registered".into());
    }
    manual_mail::validate(&settings, &credentials)?;
    manual_mail::test(settings, credentials).await
}

#[tauri::command]
async fn manual_connect(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    mut settings: manual_mail::ManualSettings,
    mut credentials: manual_mail::Credentials,
    account: Option<String>,
) -> Result<Value> {
    if !state
        .windows
        .lock()
        .map_err(err)?
        .seeds
        .contains_key(window.label())
    {
        return Err("Window is no longer registered".into());
    }
    settings.email = settings.email.trim().into();
    settings.display_name = settings.display_name.trim().into();
    if settings.protocol == "smtp" {
        credentials.incoming_password.clear();
    }
    if settings.outgoing.auth == "none" {
        credentials.smtp_password.clear();
    }
    if let Some(id) = &account {
        windows::check_owner(&state, window.label(), id)?;
        if state.account(id).await?.manual.is_none() {
            return Err("Use browser sign-in to manage this Google or Microsoft account.".into());
        }
    }
    manual_mail::validate(&settings, &credentials)?;
    manual_mail::test(settings.clone(), credentials.clone()).await?;
    let identity = json!({"email":settings.email,"protocol":settings.protocol,"incoming":settings.incoming,"outgoing":settings.outgoing});
    let id = account.clone().unwrap_or_else(|| {
        format!(
            "manual:{}",
            B64.encode(Sha256::digest(identity.to_string().as_bytes()))
        )
    });
    let a = Account {
        id,
        provider: settings.protocol.clone(),
        email: settings.email.clone(),
        client_id: String::new(),
        client_secret: String::new(),
        manual: Some(settings),
    };
    let _creation = state.window_creation.lock().await;
    if !state
        .windows
        .lock()
        .map_err(err)?
        .seeds
        .contains_key(window.label())
    {
        return Err("The connection window has closed. Open it and try again.".into());
    }
    if let Some(id) = &account {
        windows::check_owner(&state, window.label(), id)?;
    }
    let previous = state.accounts.lock().await.clone();
    let mut next = previous.clone();
    next.retain(|account| account.id != a.id);
    next.push(a.clone());
    let key = format!("{}:manual", a.id);
    let previous_credentials = mail_credentials::load::<manual_mail::Credentials>(&key).ok();
    mail_credentials::save(&key, &credentials)?;
    *state.accounts.lock().await = next;
    if let Err(error) = state.persist().await {
        *state.accounts.lock().await = previous;
        if let Some(credentials) = previous_credentials {
            let _ = mail_credentials::save(&key, &credentials);
        } else {
            let _ = mail_credentials::delete(&key);
        }
        return Err(error);
    }
    state
        .windows
        .lock()
        .map_err(err)?
        .owners
        .insert(a.id.clone(), window.label().into());
    windows::changed(window.app_handle());
    Ok(public_account(&a))
}

#[tauri::command]
fn cancel_sign_in(window: tauri::WebviewWindow, state: tauri::State<'_, Arc<App>>) -> bool {
    state.mail_sign_ins.cancel(window.label())
}
#[tauri::command]
async fn disconnect(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
) -> Result<()> {
    let _creation = state.window_creation.lock().await;
    windows::check_owner(&state, window.label(), &account)?;
    mail_credentials::delete(&account)?;
    let _ = mail_credentials::delete(&format!("{}:metadata", account));
    mail_credentials::delete(&format!("{}:manual", account))?;
    for i in 0..5 {
        let _ = entry(&format!("{account}:style:{i}"))?.delete_credential();
    }
    let _ = entry(&format!("{account}:style-index"))?.delete_credential();
    state.tokens.lock().await.remove(&account);
    state.accounts.lock().await.retain(|a| a.id != account);
    state.windows.lock().map_err(err)?.owners.remove(&account);
    state.persist().await?;
    windows::changed(window.app_handle());
    Ok(())
}
fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").into()
}
fn event_time(v: &Value, g: bool) -> String {
    let raw = v["dateTime"].as_str().or(v["date"].as_str()).unwrap_or("");
    if !g && v["timeZone"] == "UTC" && !raw.ends_with('Z') {
        format!("{raw}Z")
    } else {
        raw.into()
    }
}
fn segment(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}
const GM: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const MS: &str = "https://graph.microsoft.com/v1.0/me";
fn header(v: &Value, name: &str) -> String {
    v["payload"]["headers"]
        .as_array()
        .and_then(|h| {
            h.iter()
                .find(|h| h["name"].as_str().unwrap_or("").eq_ignore_ascii_case(name))
        })
        .map(|h| text(&h["value"]))
        .unwrap_or_default()
}
fn plain(part: &Value) -> String {
    if part["mimeType"] == "text/plain" {
        return B64
            .decode(part["body"]["data"].as_str().unwrap_or(""))
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
    }
    part["parts"]
        .as_array()
        .map(|p| {
            p.iter()
                .map(plain)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}
fn attachments(part: &Value, out: &mut Vec<Value>) {
    if let Some(id) = part["body"]["attachmentId"].as_str() {
        out.push(json!({"id":id,"name":part["filename"],"size":part["body"]["size"]}));
    }
    if let Some(p) = part["parts"].as_array() {
        for p in p {
            attachments(p, out);
        }
    }
}
fn message(v: Value, g: bool) -> Value {
    if g {
        let labels = v["labelIds"].as_array().cloned().unwrap_or_default();
        let body = plain(&v["payload"]);
        let mut files = vec![];
        attachments(&v["payload"], &mut files);
        json!({"id":v["id"],"subject":header(&v,"Subject"),"from":header(&v,"From"),"to":header(&v,"To"),"date":header(&v,"Date"),"preview":v["snippet"],"body":if body.is_empty(){"This message has no plain-text version.".into()}else{body},"unread":labels.contains(&json!("UNREAD")),"starred":labels.contains(&json!("STARRED")),"attachments":files})
    } else {
        json!({"id":v["id"],"subject":v["subject"],"from":format!("{} <{}>",text(&v["from"]["emailAddress"]["name"]),text(&v["from"]["emailAddress"]["address"])),"to":v["toRecipients"].as_array().map(|xs|xs.iter().map(|x|text(&x["emailAddress"]["address"])).collect::<Vec<_>>().join(", ")).unwrap_or_default(),"date":v["receivedDateTime"],"preview":v["bodyPreview"],"body":v["body"]["content"],"unread":!v["isRead"].as_bool().unwrap_or(false),"starred":v["flag"]["flagStatus"]=="flagged","attachments":[]})
    }
}
#[tauri::command]
async fn mail_list(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    folder: String,
    query: String,
    cursor: Option<String>,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    if let Some(settings) = a.manual.clone() {
        return manual_mail::list(settings, manual_credentials(&a)?, folder, query, cursor).await;
    }
    let g = google(&a);
    if g {
        let label = match folder.as_str() {
            "sent" => "SENT",
            "drafts" => "DRAFT",
            "trash" => "TRASH",
            "spam" => "SPAM",
            "starred" => "STARRED",
            "archive" => "",
            "inbox" => "INBOX",
            _ => return Err("Unknown folder".into()),
        };
        let mut u = url::Url::parse(&format!("{GM}/messages")).map_err(err)?;
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("maxResults", "30");
            if !label.is_empty() {
                q.append_pair("labelIds", label);
            }
            q.append_pair(
                "q",
                &if folder == "archive" {
                    format!("-in:inbox -in:trash -in:spam {query}")
                } else {
                    query
                },
            );
            if let Some(c) = cursor {
                q.append_pair("pageToken", &c);
            }
        }
        let v = state.api(&a, "GET", u.as_str(), Value::Null).await?;
        let mut items = vec![];
        if let Some(rows) = v["messages"].as_array() {
            for row in rows {
                let id = segment(row["id"].as_str().unwrap_or(""));
                items.push(message(
                    state
                        .api(
                            &a,
                            "GET",
                            &format!("{GM}/messages/{id}?format=full"),
                            Value::Null,
                        )
                        .await?,
                    true,
                ));
            }
        }
        Ok(json!({"items":items,"cursor":v["nextPageToken"]}))
    } else {
        let f = match folder.as_str() {
            "inbox" => "inbox",
            "sent" => "sentitems",
            "drafts" => "drafts",
            "trash" => "deleteditems",
            "spam" => "junkemail",
            "archive" => "archive",
            "starred" => "inbox",
            _ => return Err("Unknown folder".into()),
        };
        let offset = cursor.unwrap_or_default().parse::<u32>().unwrap_or(0);
        let mut u = url::Url::parse(&if folder == "starred" {
            format!("{MS}/messages")
        } else {
            format!("{MS}/mailFolders/{f}/messages")
        })
        .map_err(err)?;
        {
            let mut q = u.query_pairs_mut();
            q.append_pair("$top", "30")
                .append_pair("$skip", &offset.to_string());
            if query.is_empty() && folder != "starred" {
                q.append_pair("$orderby", "receivedDateTime desc");
            } else {
                q.append_pair("$search", &format!("\"{}\"", query.replace('"', "")));
            }
            if folder == "starred" {
                q.append_pair("$filter", "flag/flagStatus eq 'flagged'");
            }
        }
        let token = state.token(&a).await?;
        let v = response(
            state
                .http
                .get(u)
                .bearer_auth(token)
                .header("Prefer", "outlook.body-content-type=\"text\"")
                .send()
                .await
                .map_err(err)?,
        )
        .await?;
        Ok(
            json!({"items":v["value"].as_array().unwrap_or(&vec![]).iter().cloned().map(|v|message(v,false)).collect::<Vec<_>>(),"cursor":if v["@odata.nextLink"].is_string(){json!((offset+30).to_string())}else{Value::Null}}),
        )
    }
}
#[tauri::command]
async fn mail_action(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    id: String,
    action: String,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    if let Some(settings) = a.manual.clone() {
        return manual_mail::action(settings, manual_credentials(&a)?, id, action).await;
    }
    let id = segment(&id);
    if google(&a) {
        let (add, remove) = match action.as_str() {
            "read" => (vec![], vec!["UNREAD"]),
            "unread" => (vec!["UNREAD"], vec![]),
            "star" => (vec!["STARRED"], vec![]),
            "unstar" => (vec![], vec!["STARRED"]),
            "archive" => (vec![], vec!["INBOX"]),
            "trash" => (vec!["TRASH"], vec!["INBOX"]),
            _ => return Err("Unknown action".into()),
        };
        state
            .api(
                &a,
                "POST",
                &format!("{GM}/messages/{id}/modify"),
                json!({"addLabelIds":add,"removeLabelIds":remove}),
            )
            .await
    } else {
        match action.as_str() {
            "read" | "unread" => {
                state
                    .api(
                        &a,
                        "PATCH",
                        &format!("{MS}/messages/{id}"),
                        json!({"isRead":action=="read"}),
                    )
                    .await
            }
            "star" | "unstar" => state
                .api(
                    &a,
                    "PATCH",
                    &format!("{MS}/messages/{id}"),
                    json!({"flag":{"flagStatus":if action=="star"{"flagged"}else{"notFlagged"}}}),
                )
                .await,
            "archive" | "trash" => state
                .api(
                    &a,
                    "POST",
                    &format!("{MS}/messages/{id}/move"),
                    json!({"destinationId":if action=="archive"{"archive"}else{"deleteditems"}}),
                )
                .await,
            _ => Err("Unknown action".into()),
        }
    }
}
#[derive(Deserialize)]
struct File {
    name: String,
    data: String,
}
#[derive(Deserialize)]
struct Compose {
    to: String,
    cc: String,
    bcc: String,
    subject: String,
    body: String,
    files: Vec<File>,
}
fn addresses(s: &str) -> Result<Vec<String>> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.contains('@')
                && !s
                    .chars()
                    .any(|c| c.is_control() || c == ' ' || c == '<' || c == '>')
            {
                Ok(s.into())
            } else {
                Err(format!("Invalid email address: {s}"))
            }
        })
        .collect()
}
#[tauri::command]
async fn mail_send(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    draft: Compose,
    save: bool,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    let to = addresses(&draft.to)?;
    let cc = addresses(&draft.cc)?;
    let bcc = addresses(&draft.bcc)?;
    if !save && to.is_empty() {
        return Err("Add at least one recipient".into());
    }
    if draft.subject.contains(['\r', '\n']) {
        return Err("Subject must be a single line".into());
    }
    let attachment_limit = if a.provider == "outlook" {
        4_000_000
    } else {
        12_000_000
    };
    if draft.files.iter().map(|f| f.data.len()).sum::<usize>() > attachment_limit {
        return Err(if a.provider == "outlook" {
            "Outlook attachments exceed the 3 MB limit for direct sending"
        } else {
            "Attachments exceed the 9 MB limit"
        }
        .into());
    }
    if let Some(settings) = a.manual.clone() {
        return manual_mail::send(settings, manual_credentials(&a)?, draft, save).await;
    }
    if google(&a) {
        let mut mail = mail_builder::MessageBuilder::new()
            .from(a.email.as_str())
            .to(to.iter().map(String::as_str).collect::<Vec<_>>())
            .cc(cc.iter().map(String::as_str).collect::<Vec<_>>())
            .bcc(bcc.iter().map(String::as_str).collect::<Vec<_>>())
            .subject(draft.subject)
            .text_body(draft.body);
        for f in draft.files {
            let data = base64::engine::general_purpose::STANDARD
                .decode(f.data)
                .map_err(err)?;
            mail = mail.attachment("application/octet-stream", f.name, data);
        }
        let raw = B64.encode(mail.write_to_vec().map_err(err)?);
        state
            .api(
                &a,
                "POST",
                &format!("{GM}/{}", if save { "drafts" } else { "messages/send" }),
                if save {
                    json!({"message":{"raw":raw}})
                } else {
                    json!({"raw":raw})
                },
            )
            .await
    } else {
        let rec = |xs: Vec<String>| {
            xs.into_iter()
                .map(|e| json!({"emailAddress":{"address":e}}))
                .collect::<Vec<_>>()
        };
        let message = json!({"subject":draft.subject,"body":{"contentType":"Text","content":draft.body},"toRecipients":rec(to),"ccRecipients":rec(cc),"bccRecipients":rec(bcc),"attachments":draft.files.into_iter().map(|f|json!({"@odata.type":"#microsoft.graph.fileAttachment","name":f.name,"contentBytes":f.data})).collect::<Vec<_>>()});
        state
            .api(
                &a,
                "POST",
                &format!("{MS}/{}", if save { "messages" } else { "sendMail" }),
                if save {
                    message
                } else {
                    json!({"message":message,"saveToSentItems":true})
                },
            )
            .await
    }
}
#[tauri::command]
async fn contacts_list(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    require_groupware(&a, "Contacts")?;
    let mut result = vec![];
    let mut next = String::new();
    loop {
        let u = if google(&a) {
            format!("https://people.googleapis.com/v1/people/me/connections?personFields=names,emailAddresses,phoneNumbers,metadata&pageSize=1000&pageToken={}",segment(&next))
        } else if next.is_empty() {
            format!("{MS}/contacts?$top=100")
        } else {
            next.clone()
        };
        if !google(&a) && !u.starts_with("https://graph.microsoft.com/v1.0/") {
            return Err("Unexpected pagination URL".into());
        }
        let v = state.api(&a, "GET", &u, Value::Null).await?;
        if google(&a) {
            if let Some(rows) = v["connections"].as_array() {
                for r in rows {
                    result.push(json!({"id":r["resourceName"],"etag":r["etag"],"metadata":r["metadata"],"name":r["names"][0]["displayName"],"email":r["emailAddresses"][0]["value"],"phone":r["phoneNumbers"][0]["value"]}));
                }
            }
            next = text(&v["nextPageToken"]);
        } else {
            if let Some(rows) = v["value"].as_array() {
                for r in rows {
                    result.push(json!({"id":r["id"],"name":r["displayName"],"email":r["emailAddresses"][0]["address"],"phone":r["mobilePhone"]}));
                }
            }
            next = text(&v["@odata.nextLink"]);
        }
        if next.is_empty() {
            break;
        }
    }
    Ok(json!(result))
}
#[tauri::command]
async fn contact_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    contact: Value,
    delete: bool,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    require_groupware(&a, "Contacts")?;
    let id = text(&contact["id"]);
    if google(&a) {
        if !id.is_empty() && !id.starts_with("people/") {
            return Err("Invalid contact ID".into());
        }
        let resource = if id.is_empty() {
            String::new()
        } else {
            format!("people/{}", segment(id.trim_start_matches("people/")))
        };
        if delete {
            return state
                .api(
                    &a,
                    "DELETE",
                    &format!("https://people.googleapis.com/v1/{resource}:deleteContact"),
                    Value::Null,
                )
                .await;
        }
        let mut body = json!({"names":[{"givenName":contact["name"]}],"emailAddresses":[{"value":contact["email"]}],"phoneNumbers":[{"value":contact["phone"]}]});
        let url = if id.is_empty() {
            "https://people.googleapis.com/v1/people:createContact".into()
        } else {
            let current=state.api(&a,"GET",&format!("https://people.googleapis.com/v1/{resource}?personFields=names,emailAddresses,phoneNumbers,metadata"),Value::Null).await?;
            if current["etag"] != contact["etag"] {
                return Err(
                    "This contact changed at Google. Refresh contacts before saving.".into(),
                );
            }
            for key in ["emailAddresses", "phoneNumbers"] {
                if let Some(existing) = current[key].as_array() {
                    if existing.len() > 1 {
                        let mut values = body[key].as_array().cloned().unwrap_or_default();
                        values.extend(existing.iter().skip(1).cloned());
                        body[key] = json!(values);
                    }
                }
            }
            body["etag"] = contact["etag"].clone();
            body["metadata"] = contact["metadata"].clone();
            format!("https://people.googleapis.com/v1/{resource}:updateContact?updatePersonFields=names,emailAddresses,phoneNumbers")
        };
        state
            .api(&a, if id.is_empty() { "POST" } else { "PATCH" }, &url, body)
            .await
    } else {
        let u = if id.is_empty() {
            format!("{MS}/contacts")
        } else {
            format!("{MS}/contacts/{}", segment(&id))
        };
        if delete {
            return state.api(&a, "DELETE", &u, Value::Null).await;
        }
        let mut body = json!({"givenName":contact["name"],"displayName":contact["name"],"emailAddresses":[{"address":contact["email"]}],"mobilePhone":contact["phone"]});
        if !id.is_empty() {
            let current = state.api(&a, "GET", &u, Value::Null).await?;
            if let Some(existing) = current["emailAddresses"].as_array() {
                if existing.len() > 1 {
                    let mut values = body["emailAddresses"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    values.extend(existing.iter().skip(1).cloned());
                    body["emailAddresses"] = json!(values);
                }
            }
        }
        state
            .api(&a, if id.is_empty() { "POST" } else { "PATCH" }, &u, body)
            .await
    }
}
#[tauri::command]
async fn calendar_list(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    start: String,
    end: String,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    require_groupware(&a, "Calendar")?;
    let mut u = url::Url::parse(if google(&a) {
        "https://www.googleapis.com/calendar/v3/calendars/primary/events"
    } else {
        "https://graph.microsoft.com/v1.0/me/calendarView"
    })
    .map_err(err)?;
    if google(&a) {
        u.query_pairs_mut().extend_pairs([
            ("timeMin", start.as_str()),
            ("timeMax", end.as_str()),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", "250"),
        ]);
    } else {
        u.query_pairs_mut().extend_pairs([
            ("startDateTime", start.as_str()),
            ("endDateTime", end.as_str()),
            ("$top", "250"),
            ("$orderby", "start/dateTime"),
        ]);
    }
    let v = state.api(&a, "GET", u.as_str(), Value::Null).await?;
    let rows = if google(&a) { &v["items"] } else { &v["value"] };
    Ok(json!(rows.as_array().unwrap_or(&vec![]).iter().map(|e|json!({"id":e["id"],"title":if google(&a){&e["summary"]}else{&e["subject"]},"start":event_time(&e["start"],google(&a)),"end":event_time(&e["end"],google(&a)),"location":if google(&a){e["location"].clone()}else{e["location"]["displayName"].clone()},"link":if google(&a){e["hangoutLink"].clone()}else{e["onlineMeeting"]["joinUrl"].clone()}})).collect::<Vec<_>>()))
}
#[tauri::command]
async fn event_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    event: Value,
    delete: bool,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    require_groupware(&a, "Calendar")?;
    let id = text(&event["id"]);
    let base = if google(&a) {
        "https://www.googleapis.com/calendar/v3/calendars/primary/events"
    } else {
        "https://graph.microsoft.com/v1.0/me/events"
    };
    let url = if id.is_empty() {
        base.into()
    } else {
        format!("{base}/{}", segment(&id))
    };
    if delete {
        return state
            .api(
                &a,
                "DELETE",
                &if google(&a) {
                    format!("{url}?sendUpdates=all")
                } else {
                    url
                },
                Value::Null,
            )
            .await;
    }
    let start = text(&event["start"]);
    let end = text(&event["end"]);
    let s = chrono::DateTime::parse_from_rfc3339(&start).map_err(err)?;
    let e = chrono::DateTime::parse_from_rfc3339(&end).map_err(err)?;
    if e <= s {
        return Err("End time must be after start time".into());
    }
    let attendees = addresses(&text(&event["attendees"]))?;
    let body = if google(&a) {
        json!({"summary":event["title"],"description":event["description"],"location":event["location"],"start":{"dateTime":start},"end":{"dateTime":end},"attendees":attendees.iter().map(|e|json!({"email":e})).collect::<Vec<_>>()})
    } else {
        json!({"subject":event["title"],"body":{"contentType":"Text","content":event["description"]},"location":{"displayName":event["location"]},"start":{"dateTime":s.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S").to_string(),"timeZone":"UTC"},"end":{"dateTime":e.with_timezone(&Utc).format("%Y-%m-%dT%H:%M:%S").to_string(),"timeZone":"UTC"},"attendees":attendees.iter().map(|e|json!({"emailAddress":{"address":e},"type":"required"})).collect::<Vec<_>>()})
    };
    state
        .api(
            &a,
            if id.is_empty() { "POST" } else { "PATCH" },
            &if google(&a) {
                format!("{url}?sendUpdates=all")
            } else {
                url
            },
            body,
        )
        .await
}
#[tauri::command]
async fn rules(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    sender: Option<String>,
    remove: Option<String>,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    require_groupware(&a, "Sender rules")?;
    let base = if google(&a) {
        format!("{GM}/settings/filters")
    } else {
        format!("{MS}/mailFolders/inbox/messageRules")
    };
    if let Some(id) = remove {
        return state
            .api(
                &a,
                "DELETE",
                &format!("{base}/{}", segment(&id)),
                Value::Null,
            )
            .await;
    }
    if let Some(sender) = sender {
        let xs = addresses(&sender)?;
        if xs.len() != 1 {
            return Err("Enter one exact sender address".into());
        }
        let s = &xs[0];
        return state.api(&a,"POST",&base,if google(&a){json!({"criteria":{"from":s},"action":{"addLabelIds":["TRASH"],"removeLabelIds":["INBOX"]}})}else{json!({"displayName":format!("Yas Mail: block {s}"),"sequence":1,"isEnabled":true,"conditions":{"fromAddresses":[{"emailAddress":{"address":s}}]},"actions":{"delete":true,"stopProcessingRules":true}})}).await;
    }
    let v = state.api(&a, "GET", &base, Value::Null).await?;
    let rows = if google(&a) {
        &v["filter"]
    } else {
        &v["value"]
    };
    Ok(json!(rows.as_array().unwrap_or(&vec![]).iter().filter_map(|r|{
        if google(&a){if r["action"]["addLabelIds"].as_array().is_some_and(|xs|xs.contains(&json!("TRASH"))){Some(json!({"id":r["id"],"sender":r["criteria"]["from"]}))}else{None}}
        // Older provider rules remain recognizable after the app is renamed.
        else if ["Yas Mail: block ","GoodMail: block "].iter().any(|prefix|r["displayName"].as_str().unwrap_or("").starts_with(*prefix)){Some(json!({"id":r["id"],"sender":r["conditions"]["fromAddresses"][0]["emailAddress"]["address"]}))}else{None}
    }).collect::<Vec<_>>()))
}
#[tauri::command]
async fn attachment_list(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    id: String,
) -> Result<Value> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    if let Some(settings) = a.manual.clone() {
        return manual_mail::attachments(settings, manual_credentials(&a)?, id).await;
    }
    if google(&a) {
        let v = state
            .api(
                &a,
                "GET",
                &format!("{GM}/messages/{}?format=full", segment(&id)),
                Value::Null,
            )
            .await?;
        let mut files = vec![];
        attachments(&v["payload"], &mut files);
        Ok(json!(files))
    } else {
        let v = state
            .api(
                &a,
                "GET",
                &format!(
                    "{MS}/messages/{}/attachments?$select=id,name,size,contentType,isInline",
                    segment(&id)
                ),
                Value::Null,
            )
            .await?;
        Ok(json!(v["value"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter(|f| f["@odata.type"] == "#microsoft.graph.fileAttachment")
            .map(|f| json!({"id":f["id"],"name":f["name"],"size":f["size"]}))
            .collect::<Vec<_>>()))
    }
}
#[tauri::command]
async fn attachment_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    message_id: String,
    id: String,
    name: String,
) -> Result<bool> {
    windows::check_owner(&state, window.label(), &account)?;
    let a = state.account(&account).await?;
    let bytes = if let Some(settings) = a.manual.clone() {
        manual_mail::attachment_bytes(settings, manual_credentials(&a)?, message_id, id).await?
    } else {
        let url = format!(
            "{}/messages/{}/attachments/{}",
            if google(&a) { GM } else { MS },
            segment(&message_id),
            segment(&id)
        );
        let v = state.api(&a, "GET", &url, Value::Null).await?;
        if google(&a) {
            B64.decode(v["data"].as_str().ok_or("Attachment data missing")?)
                .map_err(err)?
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(
                    v["contentBytes"]
                        .as_str()
                        .ok_or("This attachment type cannot be downloaded")?,
                )
                .map_err(err)?
        }
    };
    windows::check_owner(&state, window.label(), &account)?;
    let safe_name = name.rsplit(['/', '\\']).next().unwrap_or("attachment");
    let file = rfd::AsyncFileDialog::new()
        .set_file_name(safe_name)
        .save_file()
        .await;
    if let Some(file) = file {
        tokio::fs::write(file.path(), bytes).await.map_err(err)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_oauth_accounts_keep_full_features_without_manual_metadata() {
        let account: Account = serde_json::from_value(json!({"id":"legacy","provider":"outlook","email":"test@example.com","client_id":"private-metadata","client_secret":"never-export"})).unwrap();
        assert!(account.manual.is_none());
        let public = public_account(&account);
        assert_eq!(public["capabilities"]["calendar"], true);
        assert_eq!(public["capabilities"]["drafts"], true);
        assert!(!public.to_string().contains("never-export"));
        assert!(!public.to_string().contains("private-metadata"));
    }
    #[test]
    fn manual_protocol_capabilities_prevent_unsupported_groupware_and_mutations() {
        for protocol in ["imap", "pop3", "smtp"] {
            let incoming = if protocol == "smtp" {
                Value::Null
            } else {
                json!({"host":"mail.example.com","port":993,"security":"tls","username":"test@example.com","auth":"password"})
            };
            let account: Account = serde_json::from_value(json!({"id":"manual:test","provider":protocol,"email":"test@example.com","client_id":"","client_secret":"","manual":{"email":"test@example.com","protocol":protocol,"incoming":incoming,"outgoing":{"host":"smtp.example.com","port":465,"security":"tls","username":"test@example.com","auth":"password"}}})).unwrap();
            let public = public_account(&account);
            assert_eq!(public["capabilities"]["contacts"], false);
            assert_eq!(public["capabilities"]["calendar"], false);
            assert_eq!(public["capabilities"]["rules"], false);
            assert_eq!(public["capabilities"]["mailSend"], true);
            assert_eq!(public["capabilities"]["mailActions"], protocol == "imap");
            assert_eq!(public["capabilities"]["drafts"], protocol == "imap");
            assert_eq!(public["capabilities"]["mailRead"], protocol != "smtp");
            assert_eq!(public["capabilities"]["mailSearch"], protocol == "imap");
            assert!(require_groupware(&account, "Contacts").is_err());
        }
    }
    #[test]
    fn rejects_header_injection() {
        assert!(addresses("person@example.com\r\nBcc: victim@example.com").is_err());
        assert!(addresses("Name <person@example.com>").is_err());
    }
    #[test]
    fn parses_recipient_lists() {
        assert_eq!(
            addresses(" a@example.com; b@example.com, ").unwrap(),
            vec!["a@example.com", "b@example.com"]
        );
    }
    #[test]
    fn extracts_nested_plain_text() {
        let part = json!({"mimeType":"multipart/alternative","parts":[{"mimeType":"text/html","body":{"data":B64.encode("<script>bad()</script>")}},{"mimeType":"text/plain","body":{"data":B64.encode("Hello world")}}]});
        assert_eq!(plain(&part), "Hello world");
    }
    #[test]
    fn normalizes_gmail_flags() {
        let v = message(
            json!({"id":"m","labelIds":["UNREAD","STARRED"],"payload":{"mimeType":"text/plain","body":{"data":B64.encode("Test")},"headers":[{"name":"subject","value":"A subject"}]}}),
            true,
        );
        assert_eq!(v["subject"], "A subject");
        assert_eq!(v["body"], "Test");
        assert_eq!(v["unread"], true);
        assert_eq!(v["starred"], true);
    }
    #[test]
    fn escapes_provider_path_ids() {
        assert_eq!(segment("a/b?c"), "a%2Fb%3Fc");
    }
}
#[tauri::command]
async fn contacts_export(content: String, format: String) -> Result<bool> {
    if !["csv", "vcf"].contains(&format.as_str()) || content.len() > 5_000_000 {
        return Err("Unsupported export format or file too large".into());
    }
    let file = rfd::AsyncFileDialog::new()
        .set_file_name(format!("Yas-Mail-contacts.{format}"))
        .add_filter("Contacts", &[format.as_str()])
        .save_file()
        .await;
    if let Some(file) = file {
        tokio::fs::write(file.path(), content.as_bytes())
            .await
            .map_err(err)?;
        Ok(true)
    } else {
        Ok(false)
    }
}
fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            app.manage(Arc::new(cloud::CloudState::new(dir.clone())?));
            let mut accounts: Vec<Account> = std::fs::read(dir.join("accounts.json"))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
                .unwrap_or_default();
            for a in &mut accounts {
                if let Ok(full) = mail_credentials::load::<Account>(&format!("{}:metadata", a.id)) {
                    *a = full;
                }
            }
            let registry = windows::WindowRegistry::new(accounts.iter().map(|a| a.id.clone()));
            app.manage(Arc::new(App {
                accounts: Mutex::new(accounts),
                tokens: Mutex::new(HashMap::new()),
                style_guard: Mutex::new(()),
                mail_sign_ins: Arc::new(mail_signin::Registry::default()),
                windows: std::sync::Mutex::new(registry),
                window_creation: Mutex::new(()),
                http: reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(45))
                    .build()?,
                dir,
            }));
            if let Some(window) = app.get_webview_window("main") {
                windows::fit_initial_window_to_work_area(&window)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(state) = window.try_state::<Arc<App>>() {
                    state.mail_sign_ins.cancel(window.label());
                }
                windows::handle_destroyed(window.app_handle(), window.label());
            }
        })
        .invoke_handler(tauri::generate_handler![
            publisher_oauth::oauth_providers,
            cancel_sign_in,
            cloud::cloud_connections,
            cloud::cloud_connect,
            cloud::cloud_disconnect,
            cloud::cloud_list,
            cloud::cloud_link,
            cloud::cloud_download,
            windows::window_context,
            windows::detach_account,
            windows::return_account,
            windows::move_account,
            windows::probe_tab_drop,
            windows::clear_tab_drop,
            windows::set_tab_drop_blocked,
            accounts,
            connect,
            manual_test,
            manual_connect,
            disconnect,
            mail_list,
            mail_action,
            mail_send,
            contacts_list,
            contact_save,
            calendar_list,
            event_save,
            rules,
            attachment_list,
            attachment_save,
            ai::ai_models,
            ai::ai_rewrite,
            ai::style_remember,
            ai::style_clear,
            contacts_export
        ])
        .run(tauri::generate_context!())
        .expect("Unable to start Yas Mail");
}
