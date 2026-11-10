//! Manual IMAP/POP3 + SMTP accounts. Passwords are supplied only by the native vault.
//! Every connection requires TLS and validates the server certificate and hostname.
use crate::{Compose, Result};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use imap::types::{Flag, NameAttribute};
use lettre::{
    address::Envelope, transport::smtp::authentication::Credentials as SmtpCredentials,
    SmtpTransport, Transport,
};
use mailparse::MailHeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

const MESSAGE_LIMIT: usize = 5_000_000;
const SESSION_LIMIT: usize = 32_000_000;
const PAGE_SIZE: usize = 20;
const IO_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub security: String,
    pub username: String,
    #[serde(default = "password_auth")]
    pub auth: String,
}
fn password_auth() -> String {
    "password".into()
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualSettings {
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    pub protocol: String,
    pub incoming: Option<ServerSettings>,
    pub outgoing: ServerSettings,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    #[serde(default)]
    pub incoming_password: String,
    #[serde(default)]
    pub smtp_password: String,
}

pub fn validate(settings: &ManualSettings, credentials: &Credentials) -> Result<()> {
    if credentials.incoming_password.len() > 4096
        || credentials.smtp_password.len() > 4096
        || credentials.incoming_password.chars().any(char::is_control)
        || credentials.smtp_password.chars().any(char::is_control)
    {
        return Err(
            "Account passwords must be at most 4096 bytes without control characters.".into(),
        );
    }
    settings
        .email
        .parse::<lettre::Address>()
        .map_err(|_| "Enter a valid email address.")?;
    if settings.email.len() > 320
        || settings.display_name.len() > 512
        || settings.display_name.chars().count() > 128
        || settings.display_name.chars().any(char::is_control)
    {
        return Err(
            "Use a display name of up to 128 characters without control characters.".into(),
        );
    }
    if !["imap", "pop3", "smtp"].contains(&settings.protocol.as_str()) {
        return Err("Choose IMAP, POP3, or SMTP only.".into());
    }
    if settings.protocol != "smtp" {
        let incoming = settings
            .incoming
            .as_ref()
            .ok_or("Enter the incoming server settings.")?;
        validate_server(incoming, true)?;
        validate_password(&credentials.incoming_password)?;
    } else if settings.incoming.is_some() {
        return Err("SMTP-only accounts must not include an incoming server.".into());
    }
    validate_server(&settings.outgoing, false)?;
    if settings.outgoing.auth == "password" {
        validate_password(&credentials.smtp_password)?;
    }
    Ok(())
}
fn validate_password(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(
            "Enter the account password or app password without control characters.".into(),
        );
    }
    Ok(())
}
fn validate_server(server: &ServerSettings, incoming: bool) -> Result<()> {
    if server.host.is_empty()
        || server.host.len() > 253
        || server
            .host
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
        || server.host.contains(['/', '\\', '@', '?', '#'])
        || server.host.contains("://")
        || server.port == 0
    {
        return Err("Enter a server hostname or IP address and a port from 1 to 65535. Do not include a URL or path.".into());
    }
    let host = server.host.trim_matches(['[', ']']);
    if host.parse::<std::net::IpAddr>().is_err()
        && (!host.is_ascii()
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            }))
    {
        return Err("Use a valid server hostname or IP address.".into());
    }
    if !["tls", "starttls"].contains(&server.security.as_str()) {
        return Err(
            "Choose SSL/TLS or required STARTTLS. Unencrypted connections are not supported."
                .into(),
        );
    }
    if !["password", "none"].contains(&server.auth.as_str())
        || incoming && server.auth != "password"
    {
        return Err("Incoming mail requires password authentication.".into());
    }
    if server.username.len() > 512
        || server.username.chars().any(char::is_control)
        || server.auth == "password" && server.username.is_empty()
    {
        return Err("Enter a valid server username.".into());
    }
    Ok(())
}

struct Bounded<S> {
    stream: S,
    remaining: usize,
    deadline: Instant,
}
impl<S> Bounded<S> {
    fn new(stream: S) -> Self {
        Self {
            stream,
            remaining: SESSION_LIMIT,
            deadline: Instant::now() + Duration::from_secs(90),
        }
    }
}
impl<S: Read> Read for Bounded<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 || Instant::now() > self.deadline {
            return Err(std::io::Error::other(
                "Mail response exceeded the transfer or time limit",
            ));
        }
        let len = buf.len().min(self.remaining);
        let n = self.stream.read(&mut buf[..len])?;
        self.remaining -= n;
        Ok(n)
    }
}
impl<S: Write> Write for Bounded<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}
type Secure = Bounded<native_tls::TlsStream<TcpStream>>;
type Imap = imap::Session<Secure>;

fn tcp(server: &ServerSettings) -> Result<TcpStream> {
    let host = server.host.trim_matches(['[', ']']);
    let addresses = (host, server.port)
        .to_socket_addrs()
        .map_err(|_| "The mail server hostname could not be resolved.")?;
    for address in addresses.take(4) {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(8)) {
            stream
                .set_read_timeout(Some(IO_TIMEOUT))
                .map_err(|_| "Could not set the mail timeout.")?;
            stream
                .set_write_timeout(Some(IO_TIMEOUT))
                .map_err(|_| "Could not set the mail timeout.")?;
            return Ok(stream);
        }
    }
    Err("Could not reach the mail server. Check the hostname, port, and network connection.".into())
}
fn tls_connector() -> Result<native_tls::TlsConnector> {
    native_tls::TlsConnector::builder()
        .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
        .build()
        .map_err(|_| "Could not initialize TLS.".into())
}
fn tls(
    server: &ServerSettings,
    stream: TcpStream,
    connector: &native_tls::TlsConnector,
) -> Result<Secure> {
    let stream=connector.connect(server.host.trim_matches(['[', ']']),stream).map_err(|_| "TLS verification failed. Check the server name, certificate, port, and encryption setting.")?;
    Ok(Bounded::new(stream))
}
fn line(reader: &mut impl Read, max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if out.len() >= max {
            return Err("The mail server returned an oversized response line.".into());
        }
        reader
            .read_exact(&mut byte)
            .map_err(|_| "The mail server closed or timed out during its response.")?;
        out.push(byte[0]);
        if out.ends_with(b"\r\n") {
            return Ok(out);
        }
    }
}
fn imap_connect(settings: &ManualSettings, credentials: &Credentials) -> Result<Imap> {
    imap_connect_using(settings, credentials, &tls_connector()?)
}
fn imap_connect_using(
    settings: &ManualSettings,
    credentials: &Credentials,
    connector: &native_tls::TlsConnector,
) -> Result<Imap> {
    let server = settings
        .incoming
        .as_ref()
        .ok_or("This account has no incoming server.")?;
    let mut raw = tcp(server)?;
    let starttls = server.security == "starttls";
    if starttls {
        let greeting = line(&mut raw, 16_384)?;
        if !greeting.starts_with(b"* OK") {
            return Err("The server did not offer an IMAP greeting.".into());
        }
        raw.write_all(b"yas000 STARTTLS\r\n")
            .map_err(|_| "Could not request IMAP STARTTLS.")?;
        let mut success = false;
        for _ in 0..100 {
            let response = line(&mut raw, 16_384)?;
            if response.starts_with(b"yas000 ") {
                success = response.starts_with(b"yas000 OK");
                break;
            }
        }
        if !success {
            return Err(
                "The IMAP server did not accept required STARTTLS. No password was sent.".into(),
            );
        }
    }
    let stream = tls(server, raw, connector)?;
    let mut client = imap::Client::new(stream);
    if !starttls {
        client
            .read_greeting()
            .map_err(|_| "The server did not offer an IMAP greeting.")?;
    }
    client.login(&server.username,&credentials.incoming_password).map_err(|_| "IMAP authentication failed. Check the username, password or app password, and server policy.".into())
}
fn smtp(settings: &ManualSettings, credentials: &Credentials) -> Result<SmtpTransport> {
    smtp_using(settings, credentials, None)
}
fn smtp_using(
    settings: &ManualSettings,
    credentials: &Credentials,
    parameters: Option<lettre::transport::smtp::client::TlsParameters>,
) -> Result<SmtpTransport> {
    let server = &settings.outgoing;
    let host = server.host.trim_matches(['[', ']']);
    let builder = if server.security == "tls" {
        SmtpTransport::relay(host)
    } else {
        SmtpTransport::starttls_relay(host)
    }
    .map_err(|_| "The SMTP server name is not valid for TLS.")?;
    let builder = if let Some(parameters) = parameters {
        builder.tls(if server.security == "tls" {
            lettre::transport::smtp::client::Tls::Wrapper(parameters)
        } else {
            lettre::transport::smtp::client::Tls::Required(parameters)
        })
    } else {
        builder
    };
    let builder = builder
        .port(server.port)
        .timeout(Some(Duration::from_secs(20)));
    let builder = if server.auth == "password" {
        builder.credentials(SmtpCredentials::new(
            server.username.clone(),
            credentials.smtp_password.clone(),
        ))
    } else {
        builder
    };
    Ok(builder.build())
}
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| "The mail task could not finish.")?
}
pub async fn test(settings: ManualSettings, credentials: Credentials) -> Result<Value> {
    validate(&settings, &credentials)?;
    blocking(move || {
        match settings.protocol.as_str() {
            "imap" => { let mut session=imap_connect(&settings,&credentials)?; session.examine("INBOX").map_err(|_| "IMAP authenticated, but INBOX could not be opened.")?; let _=session.logout(); }
            "pop3" => { let mut session=Pop::connect(&settings,&credentials)?; session.uidls()?; session.quit(); }
            _ => {}
        }
        if !smtp(&settings,&credentials)?.test_connection().map_err(|_| "SMTP connection or authentication failed. Check encryption, username, password/app password, and server policy.")? { return Err("The SMTP connection test did not succeed.".into()); }
        Ok(json!({"ok":true,"incoming":if settings.protocol=="smtp"{"not configured"}else{"connected"},"outgoing":"connected","sent":false}))
    }).await
}

fn folder(session: &mut Imap, requested: &str) -> Result<String> {
    if let Some(encoded) = requested.strip_prefix("custom:") {
        if encoded.len() > 1500 { return Err("The IMAP folder identifier is too long.".into()); }
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| "Invalid IMAP folder identifier.")?;
        let mailbox = String::from_utf8(bytes).map_err(|_| "Invalid IMAP folder identifier.")?;
        if mailbox.is_empty() || mailbox.len() > 1000 || mailbox.chars().any(char::is_control) { return Err("Invalid IMAP folder identifier.".into()); }
        return Ok(mailbox);
    }
    if requested == "inbox" || requested == "starred" {
        return Ok("INBOX".into());
    }
    let (special, candidates): (&str, &[&str]) = match requested {
        "sent" => (
            "\\Sent",
            &["Sent", "Sent Items", "Sent Messages", "INBOX.Sent"],
        ),
        "drafts" => ("\\Drafts", &["Drafts", "INBOX.Drafts"]),
        "trash" => (
            "\\Trash",
            &["Trash", "Deleted Items", "Deleted Messages", "INBOX.Trash"],
        ),
        "spam" => ("\\Junk", &["Junk", "Junk E-mail", "Spam", "INBOX.Junk"]),
        "archive" => ("\\Archive", &["Archive", "Archives", "INBOX.Archive"]),
        _ => return Err("This folder is not available for the manual account.".into()),
    };
    let names = session
        .list(None, Some("*"))
        .map_err(|_| "Could not list the IMAP folders.")?;
    for name in names.iter() {
        if name
            .attributes()
            .iter()
            .any(|a| matches!(a,NameAttribute::Custom(v) if v.eq_ignore_ascii_case(special)))
        {
            return Ok(name.name().into());
        }
    }
    for candidate in candidates {
        if let Some(name) = names
            .iter()
            .find(|name| name.name().eq_ignore_ascii_case(candidate))
        {
            return Ok(name.name().into());
        }
    }
    Err(format!("The IMAP server has no recognized {requested} folder. Configure a standard or special-use folder at your provider."))
}
#[derive(Serialize, Deserialize)]
struct ImapId {
    mailbox: String,
    validity: u32,
    uid: u32,
}
fn imap_id(mailbox: &str, validity: u32, uid: u32) -> String {
    format!(
        "imap:{}",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&ImapId {
                mailbox: mailbox.into(),
                validity,
                uid
            })
            .expect("serializable identifier")
        )
    )
}
fn parse_imap_id(value: &str) -> Result<ImapId> {
    if value.len() > 2048 {
        return Err("Invalid IMAP message identifier.".into());
    }
    let value = value
        .strip_prefix("imap:")
        .ok_or("Invalid IMAP message identifier.")?;
    let id: ImapId = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| "Invalid IMAP message identifier.")?,
    )
    .map_err(|_| "Invalid IMAP message identifier.")?;
    if id.uid == 0
        || id.validity == 0
        || id.mailbox.is_empty()
        || id.mailbox.len() > 512
        || id.mailbox.chars().any(char::is_control)
    {
        return Err("Invalid IMAP message identifier.".into());
    }
    Ok(id)
}
fn select_id(session: &mut Imap, id: &ImapId) -> Result<()> {
    let mailbox = session
        .select(&id.mailbox)
        .map_err(|_| "The message folder could not be opened.")?;
    if mailbox.uid_validity != Some(id.validity) {
        return Err(
            "This folder changed on the server. Refresh before opening or changing the message."
                .into(),
        );
    }
    Ok(())
}
fn raw_imap(session: &mut Imap, uid: u32) -> Result<(Vec<u8>, bool, bool)> {
    let uid = uid.to_string();
    let metadata = session
        .uid_fetch(&uid, "(UID RFC822.SIZE FLAGS)")
        .map_err(|_| "Could not read the IMAP message metadata.")?;
    let requested_uid = uid.parse::<u32>().expect("original numeric UID");
    let row = metadata
        .iter()
        .find(|row| row.uid == Some(requested_uid) && row.size.is_some())
        .ok_or("The message no longer exists or its metadata was incomplete.")?;
    let size = row
        .size
        .ok_or("The IMAP server did not provide a message size.")? as usize;
    if size > MESSAGE_LIMIT {
        return Err("This message exceeds the manual account preview limit of 5 MB.".into());
    }
    let unread = !row.flags().contains(&Flag::Seen);
    let starred = row.flags().contains(&Flag::Flagged);
    drop(metadata);
    let body = session
        .uid_fetch(&uid, format!("(UID BODY.PEEK[]<0.{}>)", MESSAGE_LIMIT + 1))
        .map_err(|_| "Could not download the IMAP message.")?;
    let raw = body
        .iter()
        .find(|row| row.uid == Some(requested_uid) && row.body().is_some())
        .and_then(|row| row.body())
        .ok_or("The IMAP server returned no message body.")?;
    if raw.len() > MESSAGE_LIMIT {
        return Err("This message exceeds the manual account preview limit of 5 MB.".into());
    }
    Ok((raw.to_vec(), unread, starred))
}
fn quote_search(value: &str) -> Result<String> {
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err("Use a search of up to 512 characters without control characters.".into());
    }
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}
fn offset(cursor: Option<String>) -> Result<usize> {
    match cursor {
        None => Ok(0),
        Some(value) => value
            .parse::<usize>()
            .ok()
            .filter(|v| *v <= 1_000_000)
            .ok_or("Invalid mail page cursor.".into()),
    }
}
pub async fn list(
    settings: ManualSettings,
    credentials: Credentials,
    requested: String,
    query: String,
    cursor: Option<String>,
) -> Result<Value> {
    validate(&settings, &credentials)?;
    let start = offset(cursor)?;
    blocking(move || {
        if settings.protocol=="smtp" { return Ok(json!({"items":[],"cursor":null,"notice":"This is a send-only SMTP account. It has no incoming mailbox."})); }
        if settings.protocol=="pop3" { return pop_list(&settings,&credentials,&requested,&query,start); }
        let mut session=imap_connect(&settings,&credentials)?;
        let mailbox=folder(&mut session,&requested)?;
        let selected=session.select(&mailbox).map_err(|_| "Could not open this IMAP folder.")?;
        let validity=selected.uid_validity.filter(|v|*v>0).ok_or("The IMAP server did not provide UIDVALIDITY.")?;
        let mut search=if requested=="starred" { "FLAGGED".to_string() } else { "ALL".into() };
        if !query.trim().is_empty() { search=format!("CHARSET UTF-8 {search} TEXT {}",quote_search(query.trim())?); }
        let mut uids=session.uid_search(search).map_err(|_| "The IMAP search failed. The server may not support this search text.")?.into_iter().collect::<Vec<_>>();
        uids.sort_unstable_by(|a,b|b.cmp(a));
        let mut items=vec![]; let mut bytes_read=0usize;
        for uid in uids.iter().skip(start).take(PAGE_SIZE) {
            let id=imap_id(&mailbox,validity,*uid);
            match raw_imap(&mut session,*uid) {
                Ok((raw,unread,starred))=>{bytes_read+=raw.len();items.push(parse_message(&raw,&id,unread,starred)?);},
                Err(error) if error.contains("exceeds the manual account preview limit")=>{
                    let headers=session.uid_fetch(uid.to_string(),"(UID FLAGS BODY.PEEK[HEADER]<0.65536>)").map_err(|_|"Could not read the large message headers.")?;
                    let row=headers.iter().find(|row|row.uid==Some(*uid)&&row.header().is_some()).ok_or("The message no longer exists or its headers were incomplete.")?;
                    let mut item=parse_message(row.header().unwrap_or(b""),&id,!row.flags().contains(&Flag::Seen),row.flags().contains(&Flag::Flagged))?;
                    item["body"]=json!("This message exceeds the 5 MB manual account preview limit. Open it with your provider's webmail or another client.");item["preview"]=json!("Large message · preview limited to headers");item["downloadLimited"]=json!(true);items.push(item);
                },
                Err(error)=>return Err(error),
            }
            if bytes_read>=20_000_000 {break;}
        }
        let end=start+items.len();let next=if end<uids.len(){Some(end.to_string())}else{None};
        let _=session.logout(); Ok(json!({"items":items,"cursor":next}))
    }).await
}
pub async fn action(
    settings: ManualSettings,
    credentials: Credentials,
    id: String,
    action: String,
) -> Result<Value> {
    validate(&settings, &credentials)?;
    if settings.protocol != "imap" {
        return Err("POP3 and SMTP-only accounts do not support synchronized read, star, move, or delete actions.".into());
    }
    let message = parse_imap_id(&id)?;
    blocking(move || {
        let mut session=imap_connect(&settings,&credentials)?; select_id(&mut session,&message)?;
        let uid=message.uid.to_string();
        match action.as_str() {
            "read"|"unread"|"star"|"unstar" => { let operation=match action.as_str(){"read"=>"+FLAGS.SILENT (\\Seen)","unread"=>"-FLAGS.SILENT (\\Seen)","star"=>"+FLAGS.SILENT (\\Flagged)",_=>"-FLAGS.SILENT (\\Flagged)"}; session.uid_store(&uid,operation).map_err(|_| "The IMAP server could not update this flag.")?; }
            "archive"|"trash" => { let target=folder(&mut session,if action=="archive"{"archive"}else{"trash"})?; if target==message.mailbox { return Ok(json!({"ok":true})); } session.uid_mv(&uid,target).map_err(|_| "This server could not move the message. IMAP MOVE support is required; no broad expunge was attempted.")?; }
            _=>return Err("This action is not supported by the manual account.".into()),
        }
        let _=session.logout(); Ok(json!({"ok":true}))
    }).await
}

struct Pop {
    stream: Secure,
}
impl Pop {
    fn connect(settings: &ManualSettings, credentials: &Credentials) -> Result<Self> {
        Self::connect_using(settings, credentials, &tls_connector()?)
    }
    fn connect_using(
        settings: &ManualSettings,
        credentials: &Credentials,
        connector: &native_tls::TlsConnector,
    ) -> Result<Self> {
        let server = settings
            .incoming
            .as_ref()
            .ok_or("This account has no incoming server.")?;
        let mut raw = tcp(server)?;
        let starttls = server.security == "starttls";
        if starttls {
            pop_ok(&line(&mut raw, 16_384)?)?;
            raw.write_all(b"STLS\r\n")
                .map_err(|_| "Could not request POP3 STARTTLS.")?;
            pop_ok(&line(&mut raw, 16_384)?).map_err(|_| {
                "The POP3 server did not accept required STARTTLS. No password was sent."
            })?;
        }
        let stream = tls(server, raw, connector)?;
        let mut session = Self { stream };
        if !starttls {
            pop_ok(&line(&mut session.stream, 16_384)?)?;
        }
        session.command(&format!("USER {}", server.username))?;
        session.command(&format!("PASS {}",credentials.incoming_password)).map_err(|_| "POP3 authentication failed. Check the username, app password/password, and server policy.")?;
        Ok(session)
    }
    fn command(&mut self, value: &str) -> Result<Vec<u8>> {
        if value.contains(['\r', '\n']) {
            return Err("Invalid POP3 command input.".into());
        }
        self.stream
            .write_all(format!("{value}\r\n").as_bytes())
            .map_err(|_| "Could not write to the POP3 server.")?;
        let response = line(&mut self.stream, 16_384)?;
        pop_ok(&response)?;
        Ok(response)
    }
    fn multiline(&mut self, command: &str, limit: usize) -> Result<Vec<u8>> {
        self.command(command)?;
        read_pop_multiline(&mut self.stream, limit)
    }
    fn uidls(&mut self) -> Result<Vec<(u32, String)>> {
        let rows = self.multiline("UIDL", 2_000_000).map_err(|_| {
            "The POP3 server must support UIDL for stable, non-destructive message access."
        })?;
        parse_uidls(&rows)
    }
    fn sizes(&mut self) -> Result<std::collections::HashMap<u32, usize>> {
        let rows = self.multiline("LIST", 2_000_000)?;
        let mut sizes = std::collections::HashMap::new();
        for row in String::from_utf8_lossy(&rows).lines() {
            let mut parts = row.split_whitespace();
            if let (Some(index), Some(size)) = (
                parts.next().and_then(|v| v.parse::<u32>().ok()),
                parts.next().and_then(|v| v.parse::<usize>().ok()),
            ) {
                sizes.insert(index, size);
            }
        }
        Ok(sizes)
    }
    fn retrieve(&mut self, index: u32, size: usize) -> Result<Vec<u8>> {
        if size > MESSAGE_LIMIT {
            return Err("This message exceeds the manual account preview limit of 5 MB.".into());
        }
        self.multiline(&format!("RETR {index}"), MESSAGE_LIMIT)
    }
    fn headers(&mut self, index: u32) -> Result<Option<Vec<u8>>> {
        self.stream
            .write_all(format!("TOP {index} 0\r\n").as_bytes())
            .map_err(|_| "Could not request the POP3 message headers.")?;
        let response = line(&mut self.stream, 16_384)?;
        if response.starts_with(b"-ERR") {
            return Ok(None);
        }
        pop_ok(&response)?;
        read_pop_multiline(&mut self.stream, 65_536).map(Some)
    }
    fn quit(&mut self) {
        let _ = self.command("QUIT");
    }
}
fn pop_ok(value: &[u8]) -> Result<()> {
    if value.starts_with(b"+OK") {
        Ok(())
    } else {
        Err(
            "The POP3 server rejected the request. Check account settings and server policy."
                .into(),
        )
    }
}
fn read_pop_multiline(reader: &mut impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let row = line(reader, 65_536)?;
        if row == b".\r\n" {
            return Ok(out);
        }
        let row = if row.starts_with(b"..") {
            &row[1..]
        } else {
            &row[..]
        };
        if out.len().saturating_add(row.len()) > limit {
            return Err("The POP3 response exceeded the transfer limit.".into());
        }
        out.extend_from_slice(row);
    }
}
fn parse_uidls(rows: &[u8]) -> Result<Vec<(u32, String)>> {
    let text = std::str::from_utf8(rows).map_err(|_| "The POP3 UIDL response is invalid.")?;
    let mut result = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut numbers = std::collections::HashSet::new();
    for row in text.lines() {
        let fields = row.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 2 {
            return Err("The POP3 UIDL response is invalid.".into());
        }
        let index = fields[0]
            .parse::<u32>()
            .ok()
            .filter(|v| *v > 0)
            .ok_or("The POP3 message number is invalid.")?;
        let uid = fields[1];
        if uid.len() > 512
            || !uid.bytes().all(|c| (0x21..=0x7e).contains(&c))
            || !seen.insert(uid.to_string())
            || !numbers.insert(index)
            || result.len() >= 100_000
        {
            return Err("The POP3 server did not provide unique, bounded UIDL values.".into());
        }
        result.push((index, uid.into()));
    }
    Ok(result)
}
fn pop_id(uid: &str) -> String {
    format!("pop3:{}", URL_SAFE_NO_PAD.encode(uid))
}
fn parse_pop_id(id: &str) -> Result<String> {
    if id.len() > 1024 {
        return Err("Invalid POP3 identifier.".into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(id.strip_prefix("pop3:").ok_or("Invalid POP3 identifier.")?)
        .map_err(|_| "Invalid POP3 identifier.")?;
    let uid = String::from_utf8(bytes).map_err(|_| "Invalid POP3 identifier.")?;
    if uid.is_empty()
        || uid.len() > 512
        || !uid.is_ascii()
        || uid.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("Invalid POP3 identifier.".into());
    }
    Ok(uid)
}
fn pop_list(
    settings: &ManualSettings,
    credentials: &Credentials,
    folder: &str,
    query: &str,
    start: usize,
) -> Result<Value> {
    if folder != "inbox" {
        return Ok(
            json!({"items":[],"cursor":null,"notice":"POP3 exposes only the server inbox. Messages are left on the server; folders and server flags are unavailable."}),
        );
    }
    if !query.trim().is_empty() {
        return Err("POP3 does not provide server search. Use IMAP for searchable mail.".into());
    }
    let mut session = Pop::connect(settings, credentials)?;
    let mut uids = session.uidls()?;
    uids.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let sizes = session.sizes()?;
    let mut items = vec![];
    let mut bytes_read = 0usize;
    for (index, uid) in uids.iter().skip(start).take(PAGE_SIZE) {
        let size = *sizes
            .get(index)
            .ok_or("The POP3 server did not provide a message size.")?;
        let mut item = if size > MESSAGE_LIMIT {
            let headers = session.headers(*index)?.unwrap_or_default();
            let mut item = parse_message(&headers, &pop_id(uid), false, false)?;
            item["subject"] = if item["subject"].as_str().unwrap_or("").is_empty() {
                json!("Large message")
            } else {
                item["subject"].clone()
            };
            item["body"]=json!("This message exceeds the 5 MB manual account preview limit. Open it with your provider's webmail or another client.");
            item["preview"] = json!("Large message · preview limited to headers");
            item["downloadLimited"] = json!(true);
            item
        } else {
            let raw = session.retrieve(*index, size)?;
            bytes_read += raw.len();
            parse_message(&raw, &pop_id(uid), false, false)?
        };
        item["readStateKnown"] = json!(false);
        items.push(item);
        if bytes_read >= 20_000_000 {
            break;
        }
    }
    let end = start + items.len();
    session.quit();
    Ok(
        json!({"items":items,"cursor":if end<uids.len(){Some(end.to_string())}else{None},"notice":"POP3 leaves all messages on the server. Read/star/folder state is not synchronized."}),
    )
}

fn body_text(message: &mailparse::ParsedMail<'_>, depth: usize) -> String {
    if depth > 32 {
        return String::new();
    }
    let disposition = message.get_content_disposition();
    if disposition.disposition == mailparse::DispositionType::Attachment {
        return String::new();
    }
    if message.ctype.mimetype.eq_ignore_ascii_case("text/plain") {
        return message.get_body().unwrap_or_default();
    }
    for part in &message.subparts {
        let body = body_text(part, depth + 1);
        if !body.is_empty() {
            return body;
        }
    }
    String::new()
}
fn collect_attachments(
    message: &mailparse::ParsedMail<'_>,
    path: &str,
    out: &mut Vec<Value>,
    depth: usize,
) {
    if depth > 32 || out.len() >= 100 {
        return;
    }
    let disposition = message.get_content_disposition();
    let name = disposition
        .params
        .get("filename")
        .or_else(|| message.ctype.params.get("name"));
    if let Some(name) = name {
        out.push(json!({"id":path,"name":name,"size":message.get_body_raw().map(|v|v.len()).unwrap_or(0)}));
    }
    for (index, part) in message.subparts.iter().enumerate() {
        collect_attachments(part, &format!("{path}.{index}"), out, depth + 1);
    }
}
fn parse_message(raw: &[u8], id: &str, unread: bool, starred: bool) -> Result<Value> {
    if raw.len() > MESSAGE_LIMIT {
        return Err("Message exceeds the 5 MB preview limit.".into());
    }
    let parsed =
        mailparse::parse_mail(raw).map_err(|_| "The message MIME format could not be read.")?;
    let body = body_text(&parsed, 0);
    let body = if body.is_empty() {
        "This message has no plain-text version.".into()
    } else {
        body
    };
    let date = parsed
        .headers
        .get_first_value("Date")
        .and_then(|s| mailparse::dateparse(&s).ok())
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map(|d| d.to_rfc3339())
        .unwrap_or_default();
    let mut files = vec![];
    collect_attachments(&parsed, "0", &mut files, 0);
    Ok(
        json!({"id":id,"subject":parsed.headers.get_first_value("Subject").unwrap_or_default(),"from":parsed.headers.get_first_value("From").unwrap_or_default(),"to":parsed.headers.get_first_value("To").unwrap_or_default(),"date":date,"preview":body.chars().take(140).collect::<String>(),"body":body,"unread":unread,"starred":starred,"attachments":files}),
    )
}
fn retrieve(settings: &ManualSettings, credentials: &Credentials, id: &str) -> Result<Vec<u8>> {
    match settings.protocol.as_str() {
        "imap" => {
            let id = parse_imap_id(id)?;
            let mut session = imap_connect(settings, credentials)?;
            select_id(&mut session, &id)?;
            let result = raw_imap(&mut session, id.uid).map(|v| v.0);
            let _ = session.logout();
            result
        }
        "pop3" => {
            let uid = parse_pop_id(id)?;
            let mut session = Pop::connect(settings, credentials)?;
            let index = session
                .uidls()?
                .into_iter()
                .find(|(_, value)| value == &uid)
                .ok_or("This POP3 message no longer exists. Refresh the inbox.")?
                .0;
            let size = *session
                .sizes()?
                .get(&index)
                .ok_or("The POP3 server did not provide a message size.")?;
            let result = session.retrieve(index, size);
            session.quit();
            result
        }
        _ => Err("SMTP-only accounts have no incoming messages.".into()),
    }
}
pub async fn raw_message(settings: ManualSettings, credentials: Credentials, id: String) -> Result<Vec<u8>> {
    validate(&settings, &credentials)?;
    blocking(move || retrieve(&settings, &credentials, &id)).await
}
pub async fn attachments(
    settings: ManualSettings,
    credentials: Credentials,
    message_id: String,
) -> Result<Value> {
    validate(&settings, &credentials)?;
    blocking(move || {
        let raw = retrieve(&settings, &credentials, &message_id)?;
        let message = parse_message(&raw, &message_id, false, false)?;
        Ok(message["attachments"].clone())
    })
    .await
}
pub async fn attachment_bytes(
    settings: ManualSettings,
    credentials: Credentials,
    message_id: String,
    id: String,
) -> Result<Vec<u8>> {
    validate(&settings, &credentials)?;
    if id.len() > 128 {
        return Err("Invalid attachment identifier.".into());
    }
    let path = id
        .split('.')
        .map(|s| {
            s.parse::<usize>()
                .map_err(|_| "Invalid attachment identifier.")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if path.first() != Some(&0) || path.len() > 33 {
        return Err("Invalid attachment identifier.".into());
    }
    blocking(move || {
        let raw = retrieve(&settings, &credentials, &message_id)?;
        let parsed =
            mailparse::parse_mail(&raw).map_err(|_| "The MIME message could not be read.")?;
        let mut part = &parsed;
        for index in path.iter().skip(1) {
            part = part
                .subparts
                .get(*index)
                .ok_or("Attachment no longer exists.")?;
        }
        let mut names = vec![];
        collect_attachments(part, &id, &mut names, 0);
        if names.first().and_then(|p| p["id"].as_str()) != Some(id.as_str()) {
            return Err("This message part is not a downloadable attachment.".into());
        }
        part.get_body_raw()
            .map_err(|_| "The attachment could not be decoded.".into())
    })
    .await
}

fn compose_bytes(
    settings: &ManualSettings,
    draft: Compose,
    save: bool,
) -> Result<(Vec<u8>, Option<Envelope>)> {
    let to = crate::addresses(&draft.to)?;
    let cc = crate::addresses(&draft.cc)?;
    let bcc = crate::addresses(&draft.bcc)?;
    if !save && to.is_empty() {
        return Err("Add at least one recipient.".into());
    }
    if to.len() + cc.len() + bcc.len() > 100
        || draft.subject.len() > 998
        || draft.subject.contains(['\r', '\n'])
        || draft.body.len() > 5_000_000
        || draft.files.len() > 20
    {
        return Err("Message fields exceed the manual account limits.".into());
    }
    let recipients = to
        .iter()
        .chain(cc.iter())
        .chain(bcc.iter())
        .map(|s| {
            s.parse::<lettre::Address>()
                .map_err(|_| "Invalid recipient address.")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let envelope = if save {
        None
    } else {
        Some(
            Envelope::new(
                Some(
                    settings
                        .email
                        .parse()
                        .map_err(|_| "Invalid sender address.")?,
                ),
                recipients,
            )
            .map_err(|_| "Add a valid recipient.")?,
        )
    };
    // Bcc recipients belong only in the SMTP envelope, never the transmitted headers.
    let mut message = mail_builder::MessageBuilder::new()
        .from((settings.display_name.as_str(), settings.email.as_str()))
        .to(to.iter().map(String::as_str).collect::<Vec<_>>())
        .cc(cc.iter().map(String::as_str).collect::<Vec<_>>())
        .subject(draft.subject)
        .text_body(draft.body);
    // A private server draft has no SMTP envelope, so preserve its Bcc field.
    // This branch is never used for transmitted SMTP messages.
    if save {
        message = message.bcc(bcc.iter().map(String::as_str).collect::<Vec<_>>());
    }
    let mut total = 0usize;
    for file in draft.files {
        if file.name.is_empty()
            || file.name.len() > 255
            || file.name.chars().any(char::is_control)
            || file.data.len() > 12_000_000
        {
            return Err("An attachment has an invalid filename or exceeds the size limit.".into());
        }
        let bytes = STANDARD
            .decode(file.data)
            .map_err(|_| "An attachment is not valid base64.")?;
        total = total.saturating_add(bytes.len());
        if total > 9_000_000 {
            return Err("Attachments exceed the 9 MB manual account limit.".into());
        }
        message = message.attachment("application/octet-stream", file.name, bytes);
    }
    let bytes = message
        .write_to_vec()
        .map_err(|_| "The message could not be encoded.")?;
    if bytes.len() > 18_000_000 {
        return Err("The encoded message is too large.".into());
    }
    Ok((bytes, envelope))
}
pub async fn send(
    settings: ManualSettings,
    credentials: Credentials,
    draft: Compose,
    save: bool,
) -> Result<Value> {
    validate(&settings, &credentials)?;
    if save && settings.protocol != "imap" {
        return Err("Server drafts require an IMAP account. POP3 and SMTP-only accounts cannot save server drafts.".into());
    }
    let (bytes, envelope) = compose_bytes(&settings, draft, save)?;
    blocking(move||{
        if save {let mut session=imap_connect(&settings,&credentials)?;let drafts=folder(&mut session,"drafts")?;session.append_with_flags(drafts,&bytes,&[Flag::Draft]).map_err(|_|"The IMAP draft could not be saved.")?;let _=session.logout();return Ok(json!({"saved":true}));}
        smtp(&settings,&credentials)?.send_raw(envelope.as_ref().expect("send envelope"),&bytes).map_err(|_|"SMTP did not confirm acceptance. Sending is not retried automatically; check Sent mail at your provider before retrying.")?;
        let mut stored_sent=false;
        if settings.protocol=="imap" {if let Ok(mut session)=imap_connect(&settings,&credentials){if let Ok(sent)=folder(&mut session,"sent"){stored_sent=session.append_with_flags(sent,&bytes,&[Flag::Seen]).is_ok();}let _=session.logout();}}
        Ok(json!({"accepted":true,"storedSent":stored_sent,"warning":if stored_sent{Value::Null}else{json!("SMTP accepted the message, but Yas Mail did not save a Sent copy. Your server may keep its own copy.")}}))
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> ManualSettings {
        ManualSettings {
            email: "sender@example.com".into(),
            display_name: "Example Sender".into(),
            protocol: "imap".into(),
            incoming: Some(ServerSettings {
                host: "imap.example.com".into(),
                port: 993,
                security: "tls".into(),
                username: "sender@example.com".into(),
                auth: "password".into(),
            }),
            outgoing: ServerSettings {
                host: "smtp.example.com".into(),
                port: 587,
                security: "starttls".into(),
                username: "sender@example.com".into(),
                auth: "password".into(),
            },
        }
    }
    fn credentials() -> Credentials {
        Credentials {
            incoming_password: "test incoming".into(),
            smtp_password: "test outgoing".into(),
        }
    }
    #[test]
    fn validates_tls_only_settings_and_no_passwords_in_public_config() {
        let config = settings();
        assert!(validate(&config, &credentials()).is_ok());
        let public = serde_json::to_string(&config).unwrap();
        assert!(!public.contains("test incoming"));
        let mut bad = config.clone();
        bad.incoming.as_mut().unwrap().security = "none".into();
        assert!(validate(&bad, &credentials()).is_err());
        bad = config.clone();
        bad.outgoing.host = "smtp.example.com/path".into();
        assert!(validate(&bad, &credentials()).is_err());
        bad = config.clone();
        bad.display_name = "Bad\r\nBcc: victim@example.com".into();
        assert!(validate(&bad, &credentials()).is_err());
    }
    #[test]
    fn smtp_envelope_keeps_bcc_out_of_transmitted_headers() {
        let draft = Compose {
            to: "to@example.com".into(),
            cc: "cc@example.com".into(),
            bcc: "hidden@example.com".into(),
            subject: "Hello".into(),
            body: "Message".into(),
            files: vec![],
            html: String::new(), reply_to_id: None, thread_id: None, reply_mode: None,
            internet_message_id: None, references: None, draft_id: None,
        };
        let (raw, envelope) = compose_bytes(&settings(), draft, false).unwrap();
        let raw = String::from_utf8(raw).unwrap();
        assert!(!raw.to_lowercase().contains("bcc:"));
        assert!(!raw.contains("hidden@example.com"));
        assert!(envelope
            .unwrap()
            .to()
            .iter()
            .any(|a| a.to_string() == "hidden@example.com"));
    }
    #[test]
    fn imap_private_draft_keeps_bcc_without_an_smtp_envelope() {
        let draft = Compose {
            to: "to@example.com".into(),
            cc: "".into(),
            bcc: "hidden@example.com".into(),
            subject: "Draft".into(),
            body: "Not sent".into(),
            files: vec![],
            html: String::new(), reply_to_id: None, thread_id: None, reply_mode: None,
            internet_message_id: None, references: None, draft_id: None,
        };
        let (raw, envelope) = compose_bytes(&settings(), draft, true).unwrap();
        assert!(envelope.is_none());
        let parsed = mailparse::parse_mail(&raw).unwrap();
        assert_eq!(
            parsed.headers.get_first_value("Bcc").unwrap(),
            "<hidden@example.com>"
        );
    }
    #[test]
    fn imap_identifiers_keep_folder_uidvalidity_and_uid_separate() {
        let encoded = imap_id("Sent Items", 123, 456);
        let parsed = parse_imap_id(&encoded).unwrap();
        assert_eq!(
            (parsed.mailbox, parsed.validity, parsed.uid),
            ("Sent Items".into(), 123, 456)
        );
        assert!(parse_imap_id("imap:../inbox").is_err());
        assert!(parse_imap_id(&imap_id("INBOX", 0, 1)).is_err());
        assert_eq!(
            quote_search("hello \"x\" \\ test").unwrap(),
            "\"hello \\\"x\\\" \\\\ test\""
        );
        assert!(quote_search("bad\r\nUID STORE 1").is_err());
    }
    #[test]
    fn pop_dot_unstuffing_and_uidl_are_stable_and_bounded() {
        let raw = b"Header: value\r\n\r\n..dot\r\n.\r\n";
        assert_eq!(
            read_pop_multiline(&mut &raw[..], 100).unwrap(),
            b"Header: value\r\n\r\n.dot\r\n"
        );
        assert!(read_pop_multiline(&mut &raw[..], 5).is_err());
        let rows = parse_uidls(b"1 unique-one\r\n2 unique-two\r\n").unwrap();
        assert_eq!(parse_pop_id(&pop_id(&rows[1].1)).unwrap(), "unique-two");
        assert!(parse_uidls(b"1 duplicate\r\n2 duplicate\r\n").is_err());
    }
    #[test]
    fn mime_is_plain_text_and_attachment_ids_use_part_paths() {
        let raw=b"From: Sender <sender@example.com>\r\nTo: to@example.com\r\nSubject: =?UTF-8?B?SGVsbG8=?=\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n--x\r\nContent-Type: text/html\r\n\r\n<script>bad()</script>\r\n--x\r\nContent-Type: text/plain; name=report.txt\r\nContent-Disposition: attachment; filename=report.txt\r\nContent-Transfer-Encoding: base64\r\n\r\neHl6\r\n--x--\r\n";
        let parsed = parse_message(raw, "test", true, false).unwrap();
        assert_eq!(parsed["subject"], "Hello");
        assert!(parsed["body"].as_str().unwrap().starts_with("Plain body"));
        assert!(!parsed["body"].as_str().unwrap().contains("script"));
        assert_eq!(parsed["attachments"][0]["id"], "0.2");
        assert_eq!(parsed["attachments"][0]["size"], 3);
    }
    #[test]
    fn bounded_transport_rejects_excess_bytes() {
        let mut stream = Bounded::new(&b"12345"[..]);
        stream.remaining = 3;
        let mut data = vec![0; 3];
        stream.read_exact(&mut data).unwrap();
        assert_eq!(data, b"123");
        assert!(stream.read(&mut [0]).is_err());
    }
    #[test]
    fn imap_rejected_starttls_never_sends_a_password() {
        reject_upgrade("imap");
    }
    #[test]
    fn pop_rejected_starttls_never_sends_a_password() {
        reject_upgrade("pop3");
    }
    fn reject_upgrade(protocol: &str) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let is_imap = protocol == "imap";
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .write_all(if is_imap {
                    b"* OK synthetic IMAP\r\n"
                } else {
                    b"+OK synthetic POP\r\n"
                })
                .unwrap();
            let request = line(&mut socket, 1024).unwrap();
            assert_eq!(
                request,
                if is_imap {
                    b"yas000 STARTTLS\r\n".to_vec()
                } else {
                    b"STLS\r\n".to_vec()
                }
            );
            socket
                .write_all(if is_imap {
                    b"yas000 NO no TLS\r\n"
                } else {
                    b"-ERR no TLS\r\n"
                })
                .unwrap();
            let mut next = [0u8; 1];
            assert_eq!(socket.read(&mut next).unwrap(), 0);
        });
        let mut config = settings();
        config.protocol = protocol.into();
        let incoming = config.incoming.as_mut().unwrap();
        incoming.host = "127.0.0.1".into();
        incoming.port = port;
        incoming.security = "starttls".into();
        let result = if is_imap {
            imap_connect(&config, &credentials()).map(|_| ())
        } else {
            Pop::connect(&config, &credentials()).map(|_| ())
        };
        assert!(result.unwrap_err().contains("No password was sent"));
        server.join().unwrap();
    }
    fn test_acceptor() -> native_tls::TlsAcceptor {
        let identity = native_tls::Identity::from_pkcs12(
            include_bytes!("../tests/fixtures/manual-localhost.p12"),
            "yas-mail-test-fixture",
        )
        .unwrap();
        native_tls::TlsAcceptor::new(identity).unwrap()
    }
    fn test_connector() -> native_tls::TlsConnector {
        native_tls::TlsConnector::builder()
            .add_root_certificate(
                native_tls::Certificate::from_der(include_bytes!(
                    "../tests/fixtures/manual-test-root.der"
                ))
                .unwrap(),
            )
            .min_protocol_version(Some(native_tls::Protocol::Tlsv12))
            .build()
            .unwrap()
    }
    fn local_settings(port: u16, protocol: &str) -> ManualSettings {
        let mut config = settings();
        config.protocol = protocol.into();
        let incoming = config.incoming.as_mut().unwrap();
        incoming.host = "localhost".into();
        incoming.port = port;
        incoming.security = "tls".into();
        config
    }
    #[test]
    fn imap_tls_fetch_ignores_unsolicited_other_uid_and_preserves_flags() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = test_acceptor();
        let server = std::thread::spawn(move || {
            let (raw, _) = listener.accept().unwrap();
            raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut stream = acceptor.accept(raw).unwrap();
            stream.write_all(b"* OK synthetic secure IMAP\r\n").unwrap();
            for step in 0..4 {
                let command = String::from_utf8(line(&mut stream, 4096).unwrap()).unwrap();
                let tag = command.split_whitespace().next().unwrap();
                match step {
                    0 => {
                        assert!(command.contains("LOGIN"));
                        stream
                            .write_all(format!("{tag} OK authenticated\r\n").as_bytes())
                            .unwrap();
                    }
                    1 => {
                        assert!(command.contains("UID FETCH 42"));
                        stream.write_all(format!("* 1 FETCH (UID 7 FLAGS (\\Seen) RFC822.SIZE 999)\r\n* 2 FETCH (UID 42 FLAGS (\\Flagged) RFC822.SIZE 38)\r\n{tag} OK fetched\r\n").as_bytes()).unwrap();
                    }
                    2 => {
                        assert!(command.contains("BODY.PEEK[]"));
                        let wrong = b"Subject: Wrong message\r\n\r\nWrong body";
                        let right = b"Subject: Correct message\r\n\r\nRight body";
                        stream
                            .write_all(
                                format!("* 1 FETCH (UID 7 BODY[]<0> {{{}}}\r\n", wrong.len())
                                    .as_bytes(),
                            )
                            .unwrap();
                        stream.write_all(wrong).unwrap();
                        stream
                            .write_all(
                                format!(")\r\n* 2 FETCH (UID 42 BODY[]<0> {{{}}}\r\n", right.len())
                                    .as_bytes(),
                            )
                            .unwrap();
                        stream.write_all(right).unwrap();
                        stream
                            .write_all(format!(")\r\n{tag} OK fetched\r\n").as_bytes())
                            .unwrap();
                    }
                    _ => {
                        assert!(command.contains("LOGOUT"));
                        stream
                            .write_all(format!("* BYE done\r\n{tag} OK logout\r\n").as_bytes())
                            .unwrap();
                    }
                }
            }
        });
        let config = local_settings(port, "imap");
        let mut session = imap_connect_using(&config, &credentials(), &test_connector()).unwrap();
        let (raw, unread, starred) = raw_imap(&mut session, 42).unwrap();
        assert!(String::from_utf8(raw).unwrap().contains("Correct message"));
        assert!(unread && starred);
        session.logout().unwrap();
        server.join().unwrap();
    }
    #[test]
    fn pop_tls_uidl_retrieval_uses_only_non_destructive_commands() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = test_acceptor();
        let server = std::thread::spawn(move || {
            let (raw, _) = listener.accept().unwrap();
            raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut stream = acceptor.accept(raw).unwrap();
            stream.write_all(b"+OK synthetic secure POP3\r\n").unwrap();
            let mut seen = vec![];
            loop {
                let command = String::from_utf8(line(&mut stream, 4096).unwrap()).unwrap();
                let name = command.split_whitespace().next().unwrap().to_string();
                seen.push(name.clone());
                let response = match name.as_str() {
                    "USER" | "PASS" => b"+OK authenticated\r\n".as_slice(),
                    "UIDL" => b"+OK listing\r\n1 stable-message\r\n.\r\n",
                    "LIST" => b"+OK sizes\r\n1 35\r\n.\r\n",
                    "RETR" => b"+OK message\r\nSubject: TLS POP\r\n\r\n..dot body\r\n.\r\n",
                    "QUIT" => b"+OK goodbye\r\n",
                    _ => panic!("unexpected POP command"),
                };
                stream.write_all(response).unwrap();
                if name == "QUIT" {
                    break;
                }
            }
            assert_eq!(seen, vec!["USER", "PASS", "UIDL", "LIST", "RETR", "QUIT"]);
        });
        let config = local_settings(port, "pop3");
        let mut session = Pop::connect_using(&config, &credentials(), &test_connector()).unwrap();
        assert_eq!(session.uidls().unwrap(), vec![(1, "stable-message".into())]);
        let size = session.sizes().unwrap()[&1];
        let raw = session.retrieve(1, size).unwrap();
        assert!(String::from_utf8(raw).unwrap().contains(".dot body"));
        session.quit();
        server.join().unwrap();
    }
    #[test]
    fn smtp_tls_connection_test_authenticates_without_mail_data() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let acceptor = test_acceptor();
        let server = std::thread::spawn(move || {
            let (raw, _) = listener.accept().unwrap();
            raw.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut stream = acceptor.accept(raw).unwrap();
            stream
                .write_all(b"220 localhost synthetic SMTP\r\n")
                .unwrap();
            let mut seen = vec![];
            loop {
                let command = String::from_utf8(line(&mut stream, 8192).unwrap()).unwrap();
                let name = command.split_whitespace().next().unwrap().to_string();
                seen.push(name.clone());
                let response = match name.as_str() {
                    "EHLO" => b"250-localhost\r\n250 AUTH PLAIN\r\n".as_slice(),
                    "AUTH" => b"235 2.7.0 authenticated\r\n",
                    "NOOP" => b"250 OK\r\n",
                    "QUIT" => b"221 goodbye\r\n",
                    _ => panic!("connection test attempted an unexpected SMTP command"),
                };
                stream.write_all(response).unwrap();
                if name == "QUIT" {
                    break;
                }
            }
            assert_eq!(seen, vec!["EHLO", "AUTH", "NOOP", "QUIT"]);
        });
        let mut config = settings();
        config.outgoing.host = "localhost".into();
        config.outgoing.port = port;
        config.outgoing.security = "tls".into();
        let parameters =
            lettre::transport::smtp::client::TlsParameters::builder("localhost".into())
                .add_root_certificate(
                    lettre::transport::smtp::client::Certificate::from_der(
                        include_bytes!("../tests/fixtures/manual-test-root.der").to_vec(),
                    )
                    .unwrap(),
                )
                .build()
                .unwrap();
        assert!(smtp_using(&config, &credentials(), Some(parameters))
            .unwrap()
            .test_connection()
            .unwrap());
        server.join().unwrap();
    }
}
