//! Exchange Web Services over certificate-verified HTTPS. No NTLM or certificate bypass.
use crate::{Compose, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use roxmltree::{Document, Node, ParsingOptions};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const MAX_RESPONSE: usize = 25_000_000;
const PAGE: usize = 25;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeSettings {
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    pub url: String,
    pub username: String,
    #[serde(default = "password_auth")]
    pub auth: String,
}
fn password_auth() -> String {
    "password".into()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub password: String,
}
pub fn validate(settings: &ExchangeSettings, credentials: &Credentials) -> Result<()> {
    crate::addresses(&settings.email)?;
    if !settings.email.contains('@')
        || settings.email.len() > 320
        || settings.display_name.len() > 512
        || settings.display_name.chars().any(char::is_control)
    {
        return Err("Enter a valid Exchange email address and display name".into());
    }
    verified_url(&settings.url)?;
    if settings.username.is_empty()
        || settings.username.len() > 512
        || settings.username.chars().any(char::is_control)
        || credentials.password.is_empty()
        || credentials.password.len() > 4096
        || credentials.password.chars().any(char::is_control)
    {
        return Err("Enter the Exchange username and password without control characters".into());
    }
    if settings.auth != "password" {
        return Err("This EWS connector supports password authentication over verified HTTPS. NTLM, ADFS, and certificate bypass are not supported.".into());
    }
    Ok(())
}
fn verified_url(value: &str) -> Result<url::Url> {
    let u = url::Url::parse(value).map_err(|_| "Enter the full HTTPS Exchange EWS URL")?;
    if u.scheme() != "https"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("Exchange requires an HTTPS server URL without embedded credentials, query, or fragment".into());
    }
    Ok(u)
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn content(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .and_then(|n| n.text())
        .unwrap_or("")
        .into()
}
fn descendant<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}
fn parse(value: &str) -> Result<Document<'_>> {
    if value.len() > MAX_RESPONSE {
        return Err("Exchange response exceeds the 30 MB limit".into());
    }
    Document::parse_with_options(
        value,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 200_000,
            ..Default::default()
        },
    )
    .map_err(|_| "Exchange returned invalid or unsafe XML".into())
}
fn check_response(doc: &Document<'_>) -> Result<()> {
    let root = doc.root_element();
    if root.tag_name().name() != "Envelope"
        || root.tag_name().namespace() != Some("http://schemas.xmlsoap.org/soap/envelope/")
        || !doc.descendants().any(|n| {
            n.is_element()
                && n.tag_name().name() == "Body"
                && n.tag_name().namespace() == Some("http://schemas.xmlsoap.org/soap/envelope/")
        })
    {
        return Err("The server did not return an Exchange SOAP response".into());
    }
    if !doc
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "ResponseCode")
        && !doc
            .descendants()
            .any(|n| n.is_element() && n.tag_name().name() == "Fault")
    {
        return Err("Exchange returned no operation status; success could not be confirmed".into());
    }
    for n in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ResponseCode")
    {
        if n.text() != Some("NoError") {
            let code = n.text().unwrap_or("UnknownError");
            let code = code
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(80)
                .collect::<String>();
            return Err(format!("Exchange rejected the operation ({code}). Check account permissions and server compatibility."));
        }
    }
    if doc
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "Fault")
    {
        return Err(
            "Exchange returned a SOAP fault. Check the EWS endpoint and authentication policy."
                .into(),
        );
    }
    Ok(())
}
fn envelope(body: &str) -> String {
    format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><s:Header><t:RequestServerVersion Version=\"Exchange2013_SP1\"/><t:TimeZoneContext><t:TimeZoneDefinition Id=\"UTC\"/></t:TimeZoneContext></s:Header><s:Body>{body}</s:Body></s:Envelope>")
}
fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|_| "Could not initialize Exchange HTTPS".into())
}
async fn read_response(mut response: reqwest::Response) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE as u64)
    {
        return Err("Exchange response exceeds 30 MB".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Exchange response was interrupted")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err("Exchange response exceeds 30 MB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes).map_err(|_| "Exchange returned non-UTF8 XML".into())
}
async fn soap(
    settings: &ExchangeSettings,
    credentials: &Credentials,
    operation: &str,
    body: &str,
) -> Result<String> {
    validate(settings, credentials)?;
    let response=client()?.post(verified_url(&settings.url)?).basic_auth(&settings.username,Some(&credentials.password)).header("Content-Type","text/xml; charset=utf-8").header("SOAPAction",format!("http://schemas.microsoft.com/exchange/services/2006/messages/{operation}")).body(envelope(body)).send().await.map_err(|_|"Could not connect to Exchange. Check the EWS URL, trusted certificate, network, and authentication policy.")?;
    if !response.status().is_success() {
        return Err(match response.status().as_u16(){401|403=>"Exchange authentication was rejected. The server must allow this account's password authentication over HTTPS; NTLM-only and Microsoft 365 endpoints require another connection method.",301|302|307|308=>"Exchange redirected the request. Enter the confirmed final HTTPS EWS URL; credentials are never forwarded automatically.",_=>"The Exchange server did not accept the EWS request. Check the endpoint and server availability."}.into());
    }
    let value = read_response(response).await?;
    check_response(&parse(&value)?)?;
    Ok(value)
}
pub async fn discover(email: String) -> Result<Value> {
    let values = crate::addresses(&email)?;
    if values.len() != 1 {
        return Err("Enter one Exchange email address".into());
    }
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d)
        .ok_or("Enter an email address")?;
    if domain.parse::<std::net::IpAddr>().is_ok()
        || domain.len() > 253
        || domain.split('.').any(|part| {
            part.is_empty()
                || part.starts_with('-')
                || part.ends_with('-')
                || !part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return Err("Use an email address with a valid domain".into());
    }
    let body=format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\"><Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",xml(&email));
    let mut candidates = vec![];
    for host in [domain.to_string(), format!("autodiscover.{domain}")] {
        let url = format!("https://{host}/autodiscover/autodiscover.xml");
        if let Ok(response) = client()?
            .post(&url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .body(body.clone())
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(text) = read_response(response).await {
                    if let Ok(doc) = parse(&text) {
                        for endpoint in doc
                            .descendants()
                            .filter(|n| n.is_element() && n.tag_name().name() == "EwsUrl")
                            .filter_map(|n| n.text())
                        {
                            if verified_url(endpoint).is_ok()
                                && !candidates.iter().any(|x: &Value| x["url"] == endpoint)
                            {
                                candidates.push(json!({"url":endpoint,"source":"autodiscover","authenticated":false}));
                            }
                        }
                    }
                }
            }
        }
    }
    if candidates.is_empty() {
        candidates.push(json!({"url":format!("https://{domain}/EWS/Exchange.asmx"),"source":"suggested; ask your administrator","authenticated":false}));
    }
    Ok(
        json!({"candidates":candidates,"requiresConfirmation":true,"passwordSent":false,"notice":"Confirm the server with your administrator before testing. Autodiscover probes never send a password and do not follow redirects."}),
    )
}
pub async fn test(settings: ExchangeSettings, credentials: Credentials) -> Result<Value> {
    let response=soap(&settings,&credentials,"GetFolder","<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>").await?;
    if !parse(&response)?
        .descendants()
        .any(|n| n.is_element() && n.tag_name().name() == "FolderId")
    {
        return Err("Exchange did not return an accessible inbox".into());
    }
    Ok(json!({"ok":true,"protocol":"ews","sent":false}))
}
fn folder_ref(folder: &str) -> Result<String> {
    let id = match folder {
        "inbox" | "starred" => "inbox",
        "sent" => "sentitems",
        "drafts" => "drafts",
        "trash" => "deleteditems",
        "spam" => "junkemail",
        "archive" => "archiveinbox",
        _ => {
            if let Some(id) = folder.strip_prefix("custom:") {
                if !id.is_empty() && id.len() < 4096 {
                    return Ok(format!("<t:FolderId Id=\"{}\"/>", xml(id)));
                }
            }
            return Err("Unknown Exchange folder".into());
        }
    };
    Ok(format!("<t:DistinguishedFolderId Id=\"{id}\"/>"))
}
fn item_id(id: &str) -> Result<String> {
    if id.is_empty() || id.len() > 8192 || id.chars().any(char::is_control) {
        return Err("Invalid Exchange item identifier".into());
    }
    Ok(format!("<t:ItemId Id=\"{}\"/>", xml(id)))
}
fn address(node: Node<'_, '_>) -> String {
    let name = content(node, "Name");
    let email = content(node, "EmailAddress");
    if name.is_empty() {
        email
    } else {
        format!("{name} <{email}>")
    }
}
fn address_list(node: Node<'_, '_>, name: &str) -> String {
    child(node, name)
        .into_iter()
        .flat_map(|n| n.children())
        .filter(|n| n.is_element() && n.tag_name().name() == "Mailbox")
        .map(address)
        .collect::<Vec<_>>()
        .join(", ")
}
fn strip_html(value: &str) -> String {
    let mut out = String::new();
    let mut tag = false;
    for ch in value.chars() {
        if ch == '<' {
            tag = true;
            out.push(' ');
        } else if ch == '>' {
            tag = false;
        } else if !tag {
            out.push(ch);
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
fn message(n: Node<'_, '_>) -> Value {
    let id = child(n, "ItemId");
    let body = content(n, "Body");
    let html = child(n, "Body").is_some_and(|n| n.attribute("BodyType") == Some("HTML"));
    let text_body = content(n, "TextBody");
    let plain = if !text_body.is_empty() {
        text_body
    } else if html {
        strip_html(&body)
    } else {
        body.clone()
    };
    let attachments=child(n,"Attachments").into_iter().flat_map(|n|n.children()).filter(|n|n.is_element()&&n.tag_name().name()=="FileAttachment").map(|a|json!({"id":child(a,"AttachmentId").and_then(|n|n.attribute("Id")),"name":content(a,"Name"),"size":content(a,"Size").parse::<u64>().unwrap_or(0),"contentType":content(a,"ContentType"),"inline":content(a,"IsInline")=="true","cid":content(a,"ContentId")})).collect::<Vec<_>>();
    json!({"id":id.and_then(|n|n.attribute("Id")),"changeKey":id.and_then(|n|n.attribute("ChangeKey")),"subject":content(n,"Subject"),"from":child(n,"From").and_then(|n|child(n,"Mailbox")).map(address).unwrap_or_default(),"to":address_list(n,"ToRecipients"),"cc":address_list(n,"CcRecipients"),"date":content(n,"DateTimeReceived"),"body":plain,"bodyHtml":if html{body}else{String::new()},"html":if html{content(n,"Body")}else{String::new()},"preview":plain.chars().take(160).collect::<String>(),"unread":content(n,"IsRead")!="true","starred":descendant(n,"FlagStatus").and_then(|n|n.text())==Some("Flagged"),"attachments":attachments,"internetMessageId":content(n,"InternetMessageId"),"references":content(n,"References"),"threadId":child(n,"ConversationId").and_then(|n|n.attribute("Id"))})
}
async fn get_items(
    settings: &ExchangeSettings,
    credentials: &Credentials,
    ids: &[String],
) -> Result<Vec<Value>> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    let ids = ids
        .iter()
        .map(|id| item_id(id))
        .collect::<Result<Vec<_>>>()?
        .join("");
    let response=soap(settings,credentials,"GetItem",&format!("<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>HTML</t:BodyType><t:AdditionalProperties><t:FieldURI FieldURI=\"item:TextBody\"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>")).await?;
    let doc = parse(&response)?;
    Ok(doc
        .descendants()
        .filter(|n| {
            n.is_element()
                && [
                    "Message",
                    "MeetingRequest",
                    "MeetingResponse",
                    "MeetingCancellation",
                ]
                .contains(&n.tag_name().name())
        })
        .map(message)
        .collect())
}
pub async fn list(
    settings: ExchangeSettings,
    credentials: Credentials,
    folder: String,
    query: String,
    cursor: Option<String>,
) -> Result<Value> {
    let offset = cursor.unwrap_or_default().parse::<u32>().unwrap_or(0);
    if offset > 1_000_000 || query.len() > 2000 {
        return Err("Exchange search/page exceeds the supported limit".into());
    }
    let filter = if query.is_empty() {
        String::new()
    } else {
        format!("<m:QueryString>{}</m:QueryString>", xml(&query))
    };
    let body=format!("<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned=\"{PAGE}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>{filter}<m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder><m:ParentFolderIds>{}</m:ParentFolderIds></m:FindItem>",folder_ref(&folder)?);
    let response = soap(&settings, &credentials, "FindItem", &body).await?;
    let doc = parse(&response)?;
    let ids = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "ItemId")
        .filter_map(|n| n.attribute("Id"))
        .map(str::to_owned)
        .take(PAGE)
        .collect::<Vec<_>>();
    let root = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "RootFolder");
    let next = root
        .filter(|n| n.attribute("IncludesLastItemInRange") != Some("true"))
        .map(|n| {
            n.attribute("IndexedPagingOffset")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(offset + ids.len() as u32)
                .to_string()
        });
    let mut items = get_items(&settings, &credentials, &ids).await?;
    if folder == "starred" {
        items.retain(|x| x["starred"] == true);
    }
    Ok(json!({"items":items,"cursor":next}))
}
pub async fn action(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
    action: String,
) -> Result<Value> {
    let item = item_id(&id)?;
    match action.as_str() {
        "trash" => {
            soap(&settings,&credentials,"DeleteItem",&format!("<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToNone\"><m:ItemIds>{item}</m:ItemIds></m:DeleteItem>")).await?;
        }
        "archive" => {
            soap(&settings,&credentials,"MoveItem",&format!("<m:MoveItem><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{item}</m:ItemIds></m:MoveItem>",folder_ref("archive")?)).await?;
        }
        "read" | "unread" | "star" | "unstar" => {
            let (field, value) = if ["read", "unread"].contains(&action.as_str()) {
                (
                    "message:IsRead",
                    format!("<t:IsRead>{}</t:IsRead>", action == "read"),
                )
            } else {
                (
                    "item:Flag",
                    format!(
                        "<t:Flag><t:FlagStatus>{}</t:FlagStatus></t:Flag>",
                        if action == "star" {
                            "Flagged"
                        } else {
                            "NotFlagged"
                        }
                    ),
                )
            };
            soap(&settings,&credentials,"UpdateItem",&format!("<m:UpdateItem ConflictResolution=\"AutoResolve\" MessageDisposition=\"SaveOnly\" SendMeetingInvitationsOrCancellations=\"SendToNone\"><m:ItemChanges><t:ItemChange>{item}<t:Updates><t:SetItemField><t:FieldURI FieldURI=\"{field}\"/><t:Message>{value}</t:Message></t:SetItemField></t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>")).await?;
        }
        _ => return Err("Unknown Exchange message action".into()),
    };
    Ok(json!({"ok":true}))
}
pub async fn raw_message(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
) -> Result<Vec<u8>> {
    let response=soap(&settings,&credentials,"GetItem",&format!("<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",item_id(&id)?)).await?;
    let doc = parse(&response)?;
    let mime = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "MimeContent")
        .and_then(|n| n.text())
        .ok_or("Exchange did not return MIME content")?;
    let bytes = STANDARD
        .decode(mime)
        .map_err(|_| "Exchange returned invalid MIME encoding")?;
    if bytes.len() > 25_000_000 {
        return Err("Exchange MIME export exceeds 25 MB".into());
    }
    Ok(bytes)
}
pub async fn attachments(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
) -> Result<Value> {
    let messages = get_items(&settings, &credentials, &[id]).await?;
    Ok(messages
        .first()
        .map(|x| x["attachments"].clone())
        .unwrap_or(json!([])))
}
pub async fn attachment_bytes(
    settings: ExchangeSettings,
    credentials: Credentials,
    _message_id: String,
    id: String,
) -> Result<Vec<u8>> {
    if id.is_empty() || id.len() > 8192 {
        return Err("Invalid Exchange attachment identifier".into());
    }
    let response=soap(&settings,&credentials,"GetAttachment",&format!("<m:GetAttachment><m:AttachmentShape><t:IncludeMimeContent>false</t:IncludeMimeContent></m:AttachmentShape><m:AttachmentIds><t:AttachmentId Id=\"{}\"/></m:AttachmentIds></m:GetAttachment>",xml(&id))).await?;
    let doc = parse(&response)?;
    let content = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "FileAttachment")
        .and_then(|n| child(n, "Content"))
        .and_then(|n| n.text())
        .ok_or("Exchange attachment is unavailable")?;
    let bytes = STANDARD
        .decode(content)
        .map_err(|_| "Exchange attachment encoding is invalid")?;
    if bytes.len() > 20_000_000 {
        return Err("Attachment exceeds 20 MB".into());
    }
    Ok(bytes)
}
fn recipients(tag: &str, addresses: &[String]) -> String {
    if addresses.is_empty() {
        String::new()
    } else {
        format!(
            "<t:{tag}>{}</t:{tag}>",
            addresses
                .iter()
                .map(|e| format!(
                    "<t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>",
                    xml(e)
                ))
                .collect::<String>()
        )
    }
}
pub async fn send(
    settings: ExchangeSettings,
    credentials: Credentials,
    draft: Compose,
    save: bool,
) -> Result<Value> {
    validate(&settings, &credentials)?;
    let to = crate::addresses(&draft.to)?;
    let cc = crate::addresses(&draft.cc)?;
    let bcc = crate::addresses(&draft.bcc)?;
    if !save && to.is_empty() {
        return Err("Add at least one recipient".into());
    }
    if to.len() + cc.len() + bcc.len() > 100
        || draft.subject.len() > 998
        || draft.subject.contains(['\r', '\n'])
        || draft.body.len() > 5_000_000
        || draft.files.len() > 20
    {
        return Err("Exchange message exceeds supported field limits".into());
    }
    let body_type = if draft.html.is_empty() {
        "Text"
    } else {
        "HTML"
    };
    let body = if draft.html.is_empty() {
        draft.body.clone()
    } else {
        draft.html.clone()
    };
    let recipients = format!(
        "{}{}{}",
        recipients("ToRecipients", &to),
        recipients("CcRecipients", &cc),
        recipients("BccRecipients", &bcc)
    );
    let item = if let Some(reference) = draft.reply_to_id.as_ref().filter(|id| !id.is_empty()) {
        let kind = match draft.reply_mode.as_deref() {
            Some("reply-all") => "ReplyAllToItem",
            Some("forward") => "ForwardItem",
            _ => "ReplyToItem",
        };
        format!("<t:{kind}><t:Subject>{}</t:Subject>{recipients}<t:ReferenceItemId Id=\"{}\"/><t:NewBodyContent BodyType=\"{body_type}\">{}</t:NewBodyContent></t:{kind}>",xml(&draft.subject),xml(reference),xml(&body))
    } else {
        format!("<t:Message><t:Subject>{}</t:Subject><t:Body BodyType=\"{body_type}\">{}</t:Body>{recipients}</t:Message>",xml(&draft.subject),xml(&body))
    };
    let response=soap(&settings,&credentials,"CreateItem",&format!("<m:CreateItem MessageDisposition=\"SaveOnly\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"drafts\"/></m:SavedItemFolderId><m:Items>{item}</m:Items></m:CreateItem>")).await?;
    let doc = parse(&response)?;
    let created = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "ItemId")
        .and_then(|n| n.attribute("Id"))
        .ok_or("Exchange did not return the created draft")?
        .to_owned();
    let mut total = 0usize;
    for file in draft.files {
        if file.name.is_empty() || file.name.len() > 255 || file.name.chars().any(char::is_control)
        {
            return Err("Invalid attachment filename; the draft remains saved at Exchange".into());
        }
        let data = STANDARD
            .decode(file.data)
            .map_err(|_| "Invalid attachment data; the draft remains saved at Exchange")?;
        total += data.len();
        if total > 9_000_000 {
            return Err(
                "Exchange direct-send attachments exceed 9 MB; the draft remains saved".into(),
            );
        }
        let attachment=format!("<t:FileAttachment><t:Name>{}</t:Name><t:ContentType>{}</t:ContentType><t:ContentId>{}</t:ContentId><t:IsInline>{}</t:IsInline><t:Content>{}</t:Content></t:FileAttachment>",xml(&file.name),xml(file.content_type.as_deref().unwrap_or("application/octet-stream")),xml(file.cid.as_deref().unwrap_or("")),file.inline,STANDARD.encode(data));
        soap(&settings,&credentials,"CreateAttachment",&format!("<m:CreateAttachment><m:ParentItemId Id=\"{}\"/><m:Attachments>{attachment}</m:Attachments></m:CreateAttachment>",xml(&created))).await?;
    }
    if save {
        return Ok(json!({"saved":true,"id":created}));
    }
    soap(&settings,&credentials,"SendItem",&format!("<m:SendItem SaveItemToFolder=\"true\"><m:ItemIds>{}</m:ItemIds><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId></m:SendItem>",item_id(&created)?)).await.map_err(|_|"Exchange did not confirm send acceptance. The draft may remain saved. Sending is never retried automatically; check Sent before retrying.")?;
    Ok(json!({"accepted":true}))
}
pub async fn folders(settings: ExchangeSettings, credentials: Credentials) -> Result<Value> {
    let response=soap(&settings,&credentials,"FindFolder","<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned=\"1000\" Offset=\"0\" BasePoint=\"Beginning\"/><m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>").await?;
    let doc = parse(&response)?;
    Ok(json!(doc.descendants().filter(|n|n.is_element()&&n.tag_name().name()=="Folder").filter_map(|n|child(n,"FolderId").and_then(|id|id.attribute("Id")).map(|id|json!({"id":format!("custom:{id}"),"name":content(n,"DisplayName"),"unread":content(n,"UnreadCount").parse::<u64>().unwrap_or(0),"total":content(n,"TotalCount").parse::<u64>().unwrap_or(0),"canDelete":true,"type":"folder"}))).collect::<Vec<_>>()))
}
pub async fn folder_save(
    settings: ExchangeSettings,
    credentials: Credentials,
    folder: Value,
    delete: bool,
) -> Result<Value> {
    let id = folder["id"]
        .as_str()
        .unwrap_or("")
        .trim_start_matches("custom:");
    let name = folder["name"].as_str().unwrap_or("");
    if name.len() > 255 || name.contains(['\r', '\n']) || !delete && name.trim().is_empty() {
        return Err("Enter a folder name up to 255 characters".into());
    }
    if delete {
        if id.is_empty() {
            return Err("Choose a folder".into());
        }
        soap(&settings,&credentials,"DeleteFolder",&format!("<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",xml(id))).await?;
    } else if id.is_empty() {
        let parent = folder["parent"]
            .as_str()
            .unwrap_or("")
            .trim_start_matches("custom:");
        let parent = if parent.is_empty() {
            "<t:DistinguishedFolderId Id=\"msgfolderroot\"/>".into()
        } else {
            format!("<t:FolderId Id=\"{}\"/>", xml(parent))
        };
        soap(&settings,&credentials,"CreateFolder",&format!("<m:CreateFolder><m:ParentFolderId>{parent}</m:ParentFolderId><m:Folders><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></m:Folders></m:CreateFolder>",xml(name))).await?;
    } else {
        soap(&settings,&credentials,"UpdateFolder",&format!("<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{}\"/><t:Updates><t:SetFolderField><t:FieldURI FieldURI=\"folder:DisplayName\"/><t:Folder><t:DisplayName>{}</t:DisplayName></t:Folder></t:SetFolderField></t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>",xml(id),xml(name))).await?;
    }
    Ok(json!({"ok":true}))
}
pub async fn move_message(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
    target: String,
    copy: bool,
) -> Result<Value> {
    let operation = if copy { "CopyItem" } else { "MoveItem" };
    soap(&settings,&credentials,operation,&format!("<m:{operation}><m:ToFolderId>{}</m:ToFolderId><m:ItemIds>{}</m:ItemIds></m:{operation}>",folder_ref(&target)?,item_id(&id)?)).await?;
    Ok(json!({"ok":true}))
}

fn contact(n: Node<'_, '_>, folder: &str) -> Value {
    let emails=child(n,"EmailAddresses").into_iter().flat_map(|n|n.children()).filter(|n|n.is_element()&&n.tag_name().name()=="Entry").map(|n|json!({"value":n.text().unwrap_or(""),"label":n.attribute("Key").unwrap_or("other")})).collect::<Vec<_>>();
    let phones=child(n,"PhoneNumbers").into_iter().flat_map(|n|n.children()).filter(|n|n.is_element()&&n.tag_name().name()=="Entry").map(|n|json!({"value":n.text().unwrap_or(""),"label":match n.attribute("Key"){Some("MobilePhone")=>"mobile",Some("HomePhone")=>"home",Some("BusinessPhone")=>"work",_=>"other"}})).collect::<Vec<_>>();
    let addresses=child(n,"PhysicalAddresses").into_iter().flat_map(|n|n.children()).filter(|n|n.is_element()&&n.tag_name().name()=="Entry").map(|n|json!({"street":content(n,"Street"),"city":content(n,"City"),"region":content(n,"State"),"postalCode":content(n,"PostalCode"),"country":content(n,"CountryOrRegion"),"label":n.attribute("Key").unwrap_or("other").to_lowercase()})).collect::<Vec<_>>();
    let id = child(n, "ItemId");
    json!({"id":id.and_then(|n|n.attribute("Id")),"etag":id.and_then(|n|n.attribute("ChangeKey")),"name":content(n,"DisplayName"),"givenName":content(n,"GivenName"),"familyName":content(n,"Surname"),"email":emails.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"phone":phones.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"emails":emails,"phones":phones,"company":content(n,"CompanyName"),"jobTitle":content(n,"JobTitle"),"notes":content(n,"Body"),"birthday":content(n,"Birthday").chars().take(10).collect::<String>(),"addresses":addresses,"websites":if content(n,"BusinessHomePage").is_empty(){vec![]}else{vec![json!({"value":content(n,"BusinessHomePage"),"label":"work"})]},"groups":if folder=="primary"{vec![]}else{vec![folder]},"hasPhoto":content(n,"HasPicture")=="true"})
}
pub async fn contact_groups(settings: ExchangeSettings, credentials: Credentials) -> Result<Value> {
    let response=soap(&settings,&credentials,"FindFolder","<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape><m:ParentFolderIds><t:DistinguishedFolderId Id=\"contacts\"/></m:ParentFolderIds></m:FindFolder>").await?;
    let doc = parse(&response)?;
    Ok(json!(doc.descendants().filter(|n|n.is_element()&&n.tag_name().name()=="ContactsFolder").filter_map(|n|child(n,"FolderId").and_then(|id|id.attribute("Id")).map(|id|json!({"id":id,"name":content(n,"DisplayName"),"kind":"folder","editable":true}))).collect::<Vec<_>>()))
}
pub async fn contact_group_save(
    settings: ExchangeSettings,
    credentials: Credentials,
    group: Value,
    delete: bool,
) -> Result<Value> {
    let id = group["id"].as_str().unwrap_or("");
    let name = group["name"].as_str().unwrap_or("");
    if name.len() > 256 || name.contains(['\r', '\n']) || !delete && name.trim().is_empty() {
        return Err("Enter a contact folder name".into());
    }
    if delete {
        if id.is_empty() {
            return Err("Choose a saved contact folder".into());
        }
        soap(&settings,&credentials,"DeleteFolder",&format!("<m:DeleteFolder DeleteType=\"MoveToDeletedItems\"><m:FolderIds><t:FolderId Id=\"{}\"/></m:FolderIds></m:DeleteFolder>",xml(id))).await?;
    } else if id.is_empty() {
        soap(&settings,&credentials,"CreateFolder",&format!("<m:CreateFolder><m:ParentFolderId><t:DistinguishedFolderId Id=\"contacts\"/></m:ParentFolderId><m:Folders><t:ContactsFolder><t:DisplayName>{}</t:DisplayName></t:ContactsFolder></m:Folders></m:CreateFolder>",xml(name))).await?;
    } else {
        soap(&settings,&credentials,"UpdateFolder",&format!("<m:UpdateFolder><m:FolderChanges><t:FolderChange><t:FolderId Id=\"{}\"/><t:Updates><t:SetFolderField><t:FieldURI FieldURI=\"folder:DisplayName\"/><t:ContactsFolder><t:DisplayName>{}</t:DisplayName></t:ContactsFolder></t:SetFolderField></t:Updates></t:FolderChange></m:FolderChanges></m:UpdateFolder>",xml(id),xml(name))).await?;
    }
    Ok(json!({"ok":true}))
}
pub async fn contacts(settings: ExchangeSettings, credentials: Credentials) -> Result<Value> {
    let folders = contact_groups(settings.clone(), credentials.clone()).await?;
    let mut rows = vec![];
    for folder in std::iter::once("primary").chain(
        folders
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| x["id"].as_str()),
    ) {
        let parent = if folder == "primary" {
            "<t:DistinguishedFolderId Id=\"contacts\"/>".into()
        } else {
            format!("<t:FolderId Id=\"{}\"/>", xml(folder))
        };
        let mut offset = 0;
        for _ in 0..100 {
            let response=soap(&settings,&credentials,"FindItem",&format!("<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType></m:ItemShape><m:IndexedPageItemView MaxEntriesReturned=\"200\" Offset=\"{offset}\" BasePoint=\"Beginning\"/><m:ParentFolderIds>{parent}</m:ParentFolderIds></m:FindItem>")).await?;
            let doc = parse(&response)?;
            rows.extend(
                doc.descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "Contact")
                    .map(|n| contact(n, folder)),
            );
            if rows.len() > 20_000 {
                return Err("Contacts exceed 20,000 records".into());
            }
            let root = doc
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "RootFolder");
            if root.is_none_or(|n| n.attribute("IncludesLastItemInRange") == Some("true")) {
                break;
            }
            let next = root
                .and_then(|n| n.attribute("IndexedPagingOffset"))
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(offset + 200);
            if next <= offset {
                break;
            }
            offset = next;
        }
    }
    Ok(json!(rows))
}
fn values(contact: &Value, key: &str, fallback: &str) -> Vec<Value> {
    if let Some(xs) = contact[key].as_array() {
        xs.iter()
            .filter(|x| x["value"].as_str().is_some_and(|s| !s.is_empty()))
            .cloned()
            .collect()
    } else {
        contact[fallback]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| vec![json!({"value":s,"label":"other"})])
            .unwrap_or_default()
    }
}
pub async fn save_contact(
    settings: ExchangeSettings,
    credentials: Credentials,
    c: Value,
    delete: bool,
) -> Result<Value> {
    let id = c["id"].as_str().unwrap_or("");
    if delete {
        if id.is_empty() {
            return Err("Choose a saved contact".into());
        }
        soap(&settings,&credentials,"DeleteItem",&format!("<m:DeleteItem DeleteType=\"MoveToDeletedItems\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>",item_id(id)?)).await?;
        return Ok(json!({"deleted":true}));
    }
    let name = c["name"].as_str().unwrap_or("");
    if name.trim().is_empty() || name.len() > 512 {
        return Err("Enter a contact name".into());
    }
    let emails = values(&c, "emails", "email");
    let phones = values(&c, "phones", "phone");
    if emails.len() > 3 || phones.len() > 8 {
        return Err(
            "Exchange contacts support up to three emails and eight phone entries in this editor"
                .into(),
        );
    }
    for email in &emails {
        crate::addresses(email["value"].as_str().unwrap_or(""))?;
    }
    let fields = [
        ("contacts:DisplayName", "DisplayName", name.to_string()),
        (
            "contacts:GivenName",
            "GivenName",
            c["givenName"].as_str().unwrap_or(name).into(),
        ),
        (
            "contacts:Surname",
            "Surname",
            c["familyName"].as_str().unwrap_or("").into(),
        ),
        (
            "contacts:CompanyName",
            "CompanyName",
            c["company"].as_str().unwrap_or("").into(),
        ),
        (
            "contacts:JobTitle",
            "JobTitle",
            c["jobTitle"].as_str().unwrap_or("").into(),
        ),
        (
            "item:Body",
            "Body",
            c["notes"].as_str().unwrap_or("").into(),
        ),
    ];
    let mut updates = String::new();
    for (uri, tag, value) in &fields {
        let attr = if *tag == "Body" {
            " BodyType=\"Text\""
        } else {
            ""
        };
        updates.push_str(&format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:Contact><t:{tag}{attr}>{}</t:{tag}></t:Contact></t:SetItemField>",xml(value)));
    }
    for (index, email) in emails.iter().enumerate() {
        let key = format!("EmailAddress{}", index + 1);
        updates.push_str(&format!("<t:SetItemField><t:IndexedFieldURI FieldURI=\"contacts:EmailAddress\" FieldIndex=\"{key}\"/><t:Contact><t:EmailAddresses><t:Entry Key=\"{key}\">{}</t:Entry></t:EmailAddresses></t:Contact></t:SetItemField>",xml(email["value"].as_str().unwrap_or(""))));
    }
    for (index, phone) in phones.iter().enumerate() {
        let key = match phone["label"].as_str() {
            Some("mobile") => "MobilePhone",
            Some("home") => {
                if index == 0 {
                    "HomePhone"
                } else {
                    "HomePhone2"
                }
            }
            Some("work") => {
                if index == 0 {
                    "BusinessPhone"
                } else {
                    "BusinessPhone2"
                }
            }
            _ => {
                if index == 0 {
                    "OtherTelephone"
                } else {
                    "Pager"
                }
            }
        };
        updates.push_str(&format!("<t:SetItemField><t:IndexedFieldURI FieldURI=\"contacts:PhoneNumber\" FieldIndex=\"{key}\"/><t:Contact><t:PhoneNumbers><t:Entry Key=\"{key}\">{}</t:Entry></t:PhoneNumbers></t:Contact></t:SetItemField>",xml(phone["value"].as_str().unwrap_or(""))));
    }
    let saved_id = if id.is_empty() {
        let parent = c["groups"]
            .as_array()
            .and_then(|xs| xs.first())
            .and_then(Value::as_str)
            .map(|id| format!("<t:FolderId Id=\"{}\"/>", xml(id)))
            .unwrap_or("<t:DistinguishedFolderId Id=\"contacts\"/>".into());
        let response=soap(&settings,&credentials,"CreateItem",&format!("<m:CreateItem><m:SavedItemFolderId>{parent}</m:SavedItemFolderId><m:Items><t:Contact><t:DisplayName>{}</t:DisplayName></t:Contact></m:Items></m:CreateItem>",xml(name))).await?;
        let d = parse(&response)?;
        d.descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "ItemId")
            .and_then(|n| n.attribute("Id"))
            .ok_or("Exchange did not return the contact identifier")?
            .to_owned()
    } else {
        id.into()
    };
    soap(&settings,&credentials,"UpdateItem",&format!("<m:UpdateItem ConflictResolution=\"AutoResolve\" MessageDisposition=\"SaveOnly\"><m:ItemChanges><t:ItemChange>{}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",item_id(&saved_id)?)).await?;
    let response=soap(&settings,&credentials,"GetItem",&format!("<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",item_id(&saved_id)?)).await?;
    let d = parse(&response)?;
    let n = d
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Contact")
        .ok_or("Exchange did not return the updated contact")?;
    Ok(contact(n, "primary"))
}
pub async fn contact_photo(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
) -> Result<(Vec<u8>, String)> {
    let response=soap(&settings,&credentials,"GetItem",&format!("<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",item_id(&id)?)).await?;
    let d = parse(&response)?;
    let attachment = d
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == "FileAttachment"
                && content(*n, "IsContactPhoto") == "true"
        })
        .ok_or("This contact has no photo")?;
    let attachment_id = child(attachment, "AttachmentId")
        .and_then(|n| n.attribute("Id"))
        .ok_or("Contact photo identifier missing")?
        .to_owned();
    let content_type = content(attachment, "ContentType");
    let bytes = attachment_bytes(settings, credentials, id, attachment_id).await?;
    Ok((bytes, content_type))
}
pub async fn contact_photo_save(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
    bytes: Vec<u8>,
    content_type: String,
) -> Result<Value> {
    if bytes.len() > 3_000_000 {
        return Err("Contact photo exceeds 3 MB".into());
    }
    soap(&settings,&credentials,"CreateAttachment",&format!("<m:CreateAttachment><m:ParentItemId Id=\"{}\"/><m:Attachments><t:FileAttachment><t:Name>contact-photo</t:Name><t:ContentType>{}</t:ContentType><t:IsContactPhoto>true</t:IsContactPhoto><t:Content>{}</t:Content></t:FileAttachment></m:Attachments></m:CreateAttachment>",xml(&id),xml(&content_type),STANDARD.encode(bytes))).await?;
    Ok(json!({"saved":true}))
}

fn calendar_ref(id: &str) -> String {
    if id.is_empty() || id == "primary" {
        "<t:DistinguishedFolderId Id=\"calendar\"/>".into()
    } else {
        format!("<t:FolderId Id=\"{}\"/>", xml(id))
    }
}
pub async fn calendars(settings: ExchangeSettings, credentials: Credentials) -> Result<Value> {
    let response=soap(&settings,&credentials,"FindFolder","<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape><m:ParentFolderIds><t:DistinguishedFolderId Id=\"calendar\"/></m:ParentFolderIds></m:FindFolder>").await?;
    let d = parse(&response)?;
    let mut rows = vec![json!({"id":"primary","name":"Calendar","primary":true,"canEdit":true})];
    rows.extend(d.descendants().filter(|n|n.is_element()&&n.tag_name().name()=="CalendarFolder").filter_map(|n|child(n,"FolderId").and_then(|id|id.attribute("Id")).map(|id|json!({"id":id,"name":content(n,"DisplayName"),"primary":false,"canEdit":true}))));
    Ok(json!(rows))
}
fn calendar_event(n: Node<'_, '_>, calendar: &str) -> Value {
    let id = child(n, "ItemId");
    let attendees=child(n,"RequiredAttendees").into_iter().chain(child(n,"OptionalAttendees")).flat_map(|n|n.children()).filter(|n|n.is_element()&&n.tag_name().name()=="Attendee").filter_map(|n|child(n,"Mailbox").map(|m|json!({"email":content(m,"EmailAddress"),"name":content(m,"Name"),"response":content(n,"ResponseType")}))).collect::<Vec<_>>();
    let reminder = if content(n, "ReminderIsSet") == "true" {
        vec![content(n, "ReminderMinutesBeforeStart")
            .parse::<u64>()
            .unwrap_or(15)]
    } else {
        vec![]
    };
    let all_day = content(n, "IsAllDayEvent") == "true";
    let start = content(n, "Start");
    let end = content(n, "End");
    let response = match content(n, "MyResponseType").as_str() {
        "Accept" => "accepted",
        "Tentative" => "tentative",
        "Decline" => "declined",
        _ => "needsAction",
    };
    json!({"id":id.and_then(|n|n.attribute("Id")),"etag":id.and_then(|n|n.attribute("ChangeKey")),"calendarId":calendar,"title":content(n,"Subject"),"start":if all_day{start.chars().take(10).collect::<String>()}else{start},"end":if all_day{end.chars().take(10).collect::<String>()}else{end},"allDay":all_day,"timezone":"UTC","description":content(n,"Body"),"location":content(n,"Location"),"attendees":attendees,"organizer":child(n,"Organizer").and_then(|n|child(n,"Mailbox")).map(|n|content(n,"EmailAddress")).unwrap_or_default(),"reminders":reminder,"responseStatus":response,"canEdit":content(n,"IsCancelled")!="true","onlineMeeting":false,"link":"","recurrence":recurrence_rrule(n),"isRecurring":content(n,"IsRecurring")=="true","recurringEventId":Value::Null})
}
fn recurrence_rrule(n: Node<'_, '_>) -> Value {
    let Some(r) = child(n, "Recurrence") else {
        return Value::Null;
    };
    let mut rule = String::from("RRULE:");
    let pattern = r.children().find(|n| {
        n.is_element()
            && n.tag_name().name().ends_with("Recurrence")
            && !n.tag_name().name().contains("End")
            && n.tag_name().name() != "NumberedRecurrence"
    });
    let Some(pattern) = pattern else {
        return Value::Null;
    };
    let freq = match pattern.tag_name().name() {
        "DailyRecurrence" => "DAILY",
        "WeeklyRecurrence" => "WEEKLY",
        "AbsoluteMonthlyRecurrence" => "MONTHLY",
        "AbsoluteYearlyRecurrence" => "YEARLY",
        _ => return Value::Null,
    };
    rule.push_str(&format!(
        "FREQ={freq};INTERVAL={}",
        content(pattern, "Interval").parse::<u32>().unwrap_or(1)
    ));
    if freq == "WEEKLY" {
        let days = content(pattern, "DaysOfWeek")
            .split_whitespace()
            .filter_map(|x| match x {
                "Monday" => Some("MO"),
                "Tuesday" => Some("TU"),
                "Wednesday" => Some("WE"),
                "Thursday" => Some("TH"),
                "Friday" => Some("FR"),
                "Saturday" => Some("SA"),
                "Sunday" => Some("SU"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(",");
        if !days.is_empty() {
            rule.push_str(&format!(";BYDAY={days}"));
        }
    }
    if let Some(count) = child(r, "NumberedRecurrence") {
        rule.push_str(&format!(";COUNT={}", content(count, "NumberOfOccurrences")));
    }
    if let Some(end) = child(r, "EndDateRecurrence") {
        rule.push_str(&format!(
            ";UNTIL={}",
            content(end, "EndDate")
                .chars()
                .take(10)
                .filter(|c| *c != '-')
                .collect::<String>()
        ));
    }
    json!([rule])
}
pub async fn events(
    settings: ExchangeSettings,
    credentials: Credentials,
    start: String,
    end: String,
    calendar: Option<String>,
) -> Result<Value> {
    let mut from = chrono::DateTime::parse_from_rfc3339(&start)
        .map_err(|_| "Invalid calendar start")?
        .with_timezone(&chrono::Utc);
    let to = chrono::DateTime::parse_from_rfc3339(&end)
        .map_err(|_| "Invalid calendar end")?
        .with_timezone(&chrono::Utc);
    if to <= from || (to - from).num_days() > 730 {
        return Err("Choose a period of up to two years".into());
    }
    let calendar = calendar.unwrap_or("primary".into());
    let mut rows = vec![];
    let mut seen = std::collections::HashSet::new();
    while from < to {
        let until = (from + chrono::Duration::days(7)).min(to);
        let response=soap(&settings,&credentials,"FindItem",&format!("<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape></m:ItemShape><m:CalendarView StartDate=\"{}\" EndDate=\"{}\" MaxEntriesReturned=\"1000\"/><m:ParentFolderIds>{}</m:ParentFolderIds></m:FindItem>",from.to_rfc3339(),until.to_rfc3339(),calendar_ref(&calendar))).await?;
        let d = parse(&response)?;
        if d.descendants().any(|n| {
            n.is_element()
                && n.tag_name().name() == "RootFolder"
                && n.attribute("IncludesLastItemInRange") == Some("false")
        }) {
            return Err("The Exchange calendar contains over 1000 events per week; choose a shorter period at the provider".into());
        }
        let ids = d
            .descendants()
            .filter(|n| n.is_element() && n.tag_name().name() == "ItemId")
            .filter_map(|n| n.attribute("Id"))
            .filter(|id| seen.insert((*id).to_owned()))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for chunk in ids.chunks(50) {
            let ids = chunk
                .iter()
                .map(|id| item_id(id))
                .collect::<Result<Vec<_>>>()?
                .join("");
            let response=soap(&settings,&credentials,"GetItem",&format!("<m:GetItem><m:ItemShape><t:BaseShape>AllProperties</t:BaseShape><t:BodyType>Text</t:BodyType></m:ItemShape><m:ItemIds>{ids}</m:ItemIds></m:GetItem>")).await?;
            let doc = parse(&response)?;
            rows.extend(
                doc.descendants()
                    .filter(|n| n.is_element() && n.tag_name().name() == "CalendarItem")
                    .map(|n| calendar_event(n, &calendar)),
            );
            if rows.len() > 20_000 {
                return Err("Calendar exceeds 20,000 events; choose a shorter period".into());
            }
        }
        from = until;
    }
    Ok(json!(rows))
}
fn recurrence_xml(e: &Value) -> Result<String> {
    let Some(rules) = e["recurrence"].as_array() else {
        if e["recurrence"].is_null() {
            return Ok(String::new());
        }
        return Err("Exchange recurrence must be an RRULE list".into());
    };
    if rules.is_empty() {
        return Ok(String::new());
    }
    if rules.len() != 1 {
        return Err("Exchange supports one simple RRULE in this editor".into());
    }
    let rule = rules[0]
        .as_str()
        .unwrap_or("")
        .strip_prefix("RRULE:")
        .ok_or("Invalid recurrence")?;
    let mut map = std::collections::HashMap::new();
    for part in rule.split(';') {
        let (k, v) = part.split_once('=').ok_or("Invalid recurrence")?;
        if ![
            "FREQ",
            "INTERVAL",
            "COUNT",
            "UNTIL",
            "BYDAY",
            "BYMONTHDAY",
            "BYMONTH",
        ]
        .contains(&k)
        {
            return Err("This recurrence pattern must be edited at the provider".into());
        }
        map.insert(k, v);
    }
    let start = e["start"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(10)
        .collect::<String>();
    let date = chrono::NaiveDate::parse_from_str(&start, "%Y-%m-%d")
        .map_err(|_| "Invalid recurrence start")?;
    use chrono::Datelike;
    let interval = map
        .get("INTERVAL")
        .unwrap_or(&"1")
        .parse::<u32>()
        .map_err(|_| "Invalid recurrence interval")?;
    if interval == 0 || interval > 999 {
        return Err("Recurrence interval must be 1–999".into());
    }
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let pattern=match map.get("FREQ").copied().unwrap_or(""){"DAILY"=>format!("<t:DailyRecurrence><t:Interval>{interval}</t:Interval></t:DailyRecurrence>"),"WEEKLY"=>{let days=map.get("BYDAY").map(|s|s.split(',').map(|d|match d{"MO"=>Ok("Monday"),"TU"=>Ok("Tuesday"),"WE"=>Ok("Wednesday"),"TH"=>Ok("Thursday"),"FR"=>Ok("Friday"),"SA"=>Ok("Saturday"),"SU"=>Ok("Sunday"),_=>Err("Exchange recurrence supports simple weekdays")}).collect::<std::result::Result<Vec<_>,_>>()).transpose()?.unwrap_or_else(||vec![match date.weekday(){chrono::Weekday::Mon=>"Monday",chrono::Weekday::Tue=>"Tuesday",chrono::Weekday::Wed=>"Wednesday",chrono::Weekday::Thu=>"Thursday",chrono::Weekday::Fri=>"Friday",chrono::Weekday::Sat=>"Saturday",chrono::Weekday::Sun=>"Sunday"}]);format!("<t:WeeklyRecurrence><t:Interval>{interval}</t:Interval><t:DaysOfWeek>{}</t:DaysOfWeek><t:FirstDayOfWeek>Monday</t:FirstDayOfWeek></t:WeeklyRecurrence>",days.join(" "))},"MONTHLY"=>format!("<t:AbsoluteMonthlyRecurrence><t:Interval>{interval}</t:Interval><t:DayOfMonth>{}</t:DayOfMonth></t:AbsoluteMonthlyRecurrence>",map.get("BYMONTHDAY").and_then(|s|s.parse::<u32>().ok()).filter(|d|*d>0&&*d<=31).unwrap_or(date.day())),"YEARLY"=>{let month=map.get("BYMONTH").and_then(|s|s.parse::<usize>().ok()).filter(|m|*m>0&&*m<=12).unwrap_or(date.month()as usize);format!("<t:AbsoluteYearlyRecurrence><t:DayOfMonth>{}</t:DayOfMonth><t:Month>{}</t:Month></t:AbsoluteYearlyRecurrence>",map.get("BYMONTHDAY").and_then(|s|s.parse::<u32>().ok()).filter(|d|*d>0&&*d<=31).unwrap_or(date.day()),months[month-1])},_=>return Err("Use daily, weekly, monthly, or yearly recurrence".into())};
    let range = if let Some(count) = map.get("COUNT") {
        let count = count
            .parse::<u32>()
            .map_err(|_| "Invalid recurrence count")?;
        if count == 0 || count > 999 {
            return Err("Use 1–999 recurrence occurrences".into());
        }
        format!("<t:NumberedRecurrence><t:StartDate>{start}</t:StartDate><t:NumberOfOccurrences>{count}</t:NumberOfOccurrences></t:NumberedRecurrence>")
    } else if let Some(until) = map.get("UNTIL") {
        let until =
            chrono::NaiveDate::parse_from_str(&until.chars().take(8).collect::<String>(), "%Y%m%d")
                .map_err(|_| "Invalid recurrence end")?;
        format!("<t:EndDateRecurrence><t:StartDate>{start}</t:StartDate><t:EndDate>{until}</t:EndDate></t:EndDateRecurrence>")
    } else {
        format!("<t:NoEndRecurrence><t:StartDate>{start}</t:StartDate></t:NoEndRecurrence>")
    };
    Ok(format!("<t:Recurrence>{pattern}{range}</t:Recurrence>"))
}
pub async fn save_event(
    settings: ExchangeSettings,
    credentials: Credentials,
    e: Value,
    delete: bool,
) -> Result<Value> {
    let id = e["id"].as_str().unwrap_or("");
    if delete {
        if id.is_empty() {
            return Err("Choose a saved event".into());
        }
        soap(&settings,&credentials,"DeleteItem",&format!("<m:DeleteItem DeleteType=\"MoveToDeletedItems\" SendMeetingCancellations=\"SendToAllAndSaveCopy\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>",item_id(id)?)).await?;
        return Ok(json!({"deleted":true}));
    }
    if e["onlineMeeting"] == true {
        return Err("Automatic Teams/Meet links require the Microsoft 365 or Google connection. An on-premises EWS account can include an existing meeting URL in the description.".into());
    }
    let title = e["title"].as_str().unwrap_or("");
    if title.trim().is_empty() || title.len() > 1000 {
        return Err("Enter an event title up to 1000 characters".into());
    }
    let all_day = e["allDay"] == true;
    let parse_date = |key: &str| -> Result<chrono::DateTime<chrono::Utc>> {
        let value = e[key].as_str().unwrap_or("");
        if all_day {
            Ok(chrono::NaiveDate::parse_from_str(
                &value.chars().take(10).collect::<String>(),
                "%Y-%m-%d",
            )
            .map_err(|_| "Invalid all-day date")?
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc())
        } else {
            Ok(chrono::DateTime::parse_from_rfc3339(value)
                .map_err(|_| "Event times require an offset")?
                .with_timezone(&chrono::Utc))
        }
    };
    let start = parse_date("start")?;
    let end = parse_date("end")?;
    if end <= start {
        return Err("Event end must be after start".into());
    }
    let attendees = if let Some(xs) = e["attendees"].as_array() {
        xs.iter()
            .map(|x| {
                x.as_str()
                    .unwrap_or_else(|| x["email"].as_str().unwrap_or(""))
            })
            .collect::<Vec<_>>()
            .join(",")
    } else {
        e["attendees"].as_str().unwrap_or("").into()
    };
    let attendees = crate::addresses(&attendees)?;
    if attendees.len() > 200 {
        return Err("Use up to 200 attendees".into());
    }
    let required=format!("<t:RequiredAttendees>{}</t:RequiredAttendees>",attendees.iter().map(|email|format!("<t:Attendee><t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox></t:Attendee>",xml(email))).collect::<String>());
    let reminder = e["reminders"]
        .as_array()
        .and_then(|xs| xs.first())
        .and_then(Value::as_u64)
        .filter(|m| *m <= 40320);
    let fields = [
        ("item:Subject", "Subject", xml(title)),
        (
            "item:Body",
            "Body",
            xml(e["description"].as_str().unwrap_or("")),
        ),
        ("calendar:Start", "Start", start.to_rfc3339()),
        ("calendar:End", "End", end.to_rfc3339()),
        (
            "calendar:IsAllDayEvent",
            "IsAllDayEvent",
            all_day.to_string(),
        ),
        (
            "calendar:Location",
            "Location",
            xml(e["location"].as_str().unwrap_or("")),
        ),
        (
            "item:ReminderIsSet",
            "ReminderIsSet",
            reminder.is_some().to_string(),
        ),
        (
            "item:ReminderMinutesBeforeStart",
            "ReminderMinutesBeforeStart",
            reminder.unwrap_or(15).to_string(),
        ),
    ];
    let recurrence = recurrence_xml(&e)?;
    let result = if id.is_empty() {
        let calendar = e["calendarId"].as_str().unwrap_or("primary");
        let item=format!("<t:CalendarItem><t:Subject>{}</t:Subject><t:Body BodyType=\"Text\">{}</t:Body><t:ReminderIsSet>{}</t:ReminderIsSet><t:ReminderMinutesBeforeStart>{}</t:ReminderMinutesBeforeStart><t:Start>{}</t:Start><t:End>{}</t:End><t:IsAllDayEvent>{all_day}</t:IsAllDayEvent><t:Location>{}</t:Location>{required}{recurrence}</t:CalendarItem>",xml(title),xml(e["description"].as_str().unwrap_or("")),reminder.is_some(),reminder.unwrap_or(15),start.to_rfc3339(),end.to_rfc3339(),xml(e["location"].as_str().unwrap_or("")));
        soap(&settings,&credentials,"CreateItem",&format!("<m:CreateItem SendMeetingInvitations=\"SendToAllAndSaveCopy\"><m:SavedItemFolderId>{}</m:SavedItemFolderId><m:Items>{item}</m:Items></m:CreateItem>",calendar_ref(calendar))).await?
    } else {
        let mut updates=fields.iter().map(|(uri,tag,value)|format!("<t:SetItemField><t:FieldURI FieldURI=\"{uri}\"/><t:CalendarItem><t:{tag}{}>{value}</t:{tag}></t:CalendarItem></t:SetItemField>",if *tag=="Body"{" BodyType=\"Text\""}else{""})).collect::<String>();
        updates.push_str(&format!("<t:SetItemField><t:FieldURI FieldURI=\"calendar:RequiredAttendees\"/><t:CalendarItem>{required}</t:CalendarItem></t:SetItemField>"));
        if e.get("recurrence").is_some() {
            if recurrence.is_empty() {
                updates.push_str("<t:DeleteItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/></t:DeleteItemField>");
            } else {
                updates.push_str(&format!("<t:SetItemField><t:FieldURI FieldURI=\"calendar:Recurrence\"/><t:CalendarItem>{recurrence}</t:CalendarItem></t:SetItemField>"));
            }
        }
        soap(&settings,&credentials,"UpdateItem",&format!("<m:UpdateItem ConflictResolution=\"AutoResolve\" SendMeetingInvitationsOrCancellations=\"SendToAllAndSaveCopy\"><m:ItemChanges><t:ItemChange>{}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",item_id(id)?)).await?
    };
    let d = parse(&result)?;
    let saved = d
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "ItemId")
        .and_then(|n| n.attribute("Id"))
        .unwrap_or(id);
    let mut result = e.clone();
    result["id"] = json!(saved);
    Ok(result)
}
pub async fn respond_event(
    settings: ExchangeSettings,
    credentials: Credentials,
    id: String,
    response: String,
) -> Result<Value> {
    let kind = match response.as_str() {
        "accepted" => "AcceptItem",
        "tentative" => "TentativelyAcceptItem",
        "declined" => "DeclineItem",
        _ => return Err("Choose Accept, Tentative, or Decline".into()),
    };
    soap(&settings,&credentials,"CreateItem",&format!("<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:Items><t:{kind}><t:ReferenceItemId Id=\"{}\"/></t:{kind}></m:Items></m:CreateItem>",xml(&id))).await?;
    Ok(json!({"response":response}))
}

fn first_string(node:Node<'_,'_>,property:&str)->String{child(node,property).and_then(|n|child(n,"String")).and_then(|n|n.text()).unwrap_or("").into()}
fn normalize_rule(n:Node<'_,'_>)->Value{let conditions=child(n,"Conditions");let actions=child(n,"Actions");let supported_conditions=["ContainsSenderStrings","ContainsRecipientStrings","ContainsSubjectStrings","ContainsBodyStrings","HasAttachments"];let supported_actions=["MoveToFolder","MarkAsRead","Delete","StopProcessingRules"];let editable=conditions.is_none_or(|n|n.children().filter(Node::is_element).all(|n|supported_conditions.contains(&n.tag_name().name())&&n.children().filter(|n|n.is_element()&&n.tag_name().name()=="String").count()<=1))&&actions.is_none_or(|n|n.children().filter(Node::is_element).all(|n|supported_actions.contains(&n.tag_name().name())))&&child(n,"Exceptions").is_none_or(|n|!n.children().any(|n|n.is_element()))&&content(n,"IsNotSupported")!="true";let folder=actions.and_then(|n|child(n,"MoveToFolder")).and_then(|n|child(n,"FolderId")).and_then(|n|n.attribute("Id")).map(|id|format!("custom:{id}")).unwrap_or_default();json!({"id":content(n,"RuleId"),"name":content(n,"DisplayName"),"enabled":content(n,"IsEnabled")=="true","priority":content(n,"Priority").parse::<u32>().unwrap_or(1),"editable":editable,"stopProcessing":actions.is_some_and(|n|content(n,"StopProcessingRules")=="true"),"conditions":{"from":conditions.map(|n|first_string(n,"ContainsSenderStrings")).unwrap_or_default(),"to":conditions.map(|n|first_string(n,"ContainsRecipientStrings")).unwrap_or_default(),"subject":conditions.map(|n|first_string(n,"ContainsSubjectStrings")).unwrap_or_default(),"contains":conditions.map(|n|first_string(n,"ContainsBodyStrings")).unwrap_or_default(),"hasAttachment":conditions.is_some_and(|n|content(n,"HasAttachments")=="true")},"actions":{"folder":folder,"markRead":actions.is_some_and(|n|content(n,"MarkAsRead")=="true"),"trash":actions.is_some_and(|n|content(n,"Delete")=="true"),"star":false}})}
pub async fn rules(settings:ExchangeSettings,credentials:Credentials)->Result<Value>{let response=soap(&settings,&credentials,"GetInboxRules","<m:GetInboxRules/> ").await?;let d=parse(&response)?;Ok(json!(d.descendants().filter(|n|n.is_element()&&n.tag_name().name()=="Rule").map(normalize_rule).collect::<Vec<_>>()))}
fn rule_xml(rule:&Value,priority:u64,stop:bool)->Result<String>{let id=rule["id"].as_str().unwrap_or("");let name=rule["name"].as_str().unwrap_or("");if name.trim().is_empty()||name.len()>256||name.chars().any(char::is_control){return Err("Enter a rule name up to 256 characters".into());}if rule["actions"]["star"]==true{return Err("Exchange EWS inbox rules cannot apply a follow-up star through this editor".into());}let mut conditions=String::new();for(key,tag)in[("from","ContainsSenderStrings"),("to","ContainsRecipientStrings"),("subject","ContainsSubjectStrings"),("contains","ContainsBodyStrings")]{if let Some(value)=rule["conditions"][key].as_str().filter(|s|!s.trim().is_empty()){if value.len()>2000||value.chars().any(char::is_control){return Err("Rule conditions must be single lines up to 2000 characters".into());}conditions.push_str(&format!("<t:{tag}><t:String>{}</t:String></t:{tag}>",xml(value)));}}if rule["conditions"]["hasAttachment"]==true{conditions.push_str("<t:HasAttachments>true</t:HasAttachments>");}if conditions.is_empty(){return Err("Add at least one rule condition".into());}let mut actions=String::new();if rule["actions"]["trash"]==true{actions.push_str("<t:Delete>true</t:Delete>");}if rule["actions"]["markRead"]==true{actions.push_str("<t:MarkAsRead>true</t:MarkAsRead>");}if let Some(folder)=rule["actions"]["folder"].as_str().filter(|s|!s.is_empty()){actions.push_str(&format!("<t:MoveToFolder>{}</t:MoveToFolder>",folder_ref(folder)?));}if actions.is_empty(){return Err("Choose at least one rule action".into());}actions.push_str(&format!("<t:StopProcessingRules>{stop}</t:StopProcessingRules>"));Ok(format!("<t:Rule>{}<t:DisplayName>{}</t:DisplayName><t:Priority>{priority}</t:Priority><t:IsEnabled>{}</t:IsEnabled><t:Conditions>{conditions}</t:Conditions><t:Actions>{actions}</t:Actions></t:Rule>",if id.is_empty(){String::new()}else{format!("<t:RuleId>{}</t:RuleId>",xml(id))},xml(name),rule["enabled"].as_bool().unwrap_or(true)))}
pub async fn rule_save(settings:ExchangeSettings,credentials:Credentials,rule:Value,delete:bool)->Result<Value>{let current=rules(settings.clone(),credentials.clone()).await?;let id=rule["id"].as_str().unwrap_or("");let existing=current.as_array().into_iter().flatten().find(|x|x["id"]==id);if !id.is_empty()&&existing.is_none(){return Err("This Exchange rule changed or no longer exists. Refresh before editing.".into());}if !delete&&existing.is_some_and(|x|x["editable"]!=true){return Err("This rule contains conditions or actions not supported by this editor. Edit it in Outlook to preserve its behavior.".into());}let operation=if delete{if id.is_empty(){return Err("Choose a saved rule".into());}format!("<t:DeleteRuleOperation><t:RuleId>{}</t:RuleId></t:DeleteRuleOperation>",xml(id))}else{let priority=existing.and_then(|x|x["priority"].as_u64()).unwrap_or_else(||current.as_array().into_iter().flatten().filter_map(|x|x["priority"].as_u64()).max().unwrap_or(0)+1);let stop=rule["stopProcessing"].as_bool().or_else(||existing.and_then(|x|x["stopProcessing"].as_bool())).unwrap_or(true);let value=rule_xml(&rule,priority,stop)?;let operation=if id.is_empty(){"CreateRuleOperation"}else{"SetRuleOperation"};format!("<t:{operation}>{value}</t:{operation}>")};soap(&settings,&credentials,"UpdateInboxRules",&format!("<m:UpdateInboxRules><m:RemoveOutlookRuleBlob>false</m:RemoveOutlookRuleBlob><m:Operations>{operation}</m:Operations></m:UpdateInboxRules>")).await.map_err(|e|format!("{e} Existing Outlook-only rule data is never removed automatically."))?;Ok(json!({"ok":true}))}

#[cfg(test)]
mod tests {
    use super::*;
    fn settings() -> ExchangeSettings {
        ExchangeSettings {
            email: "test@example.invalid".into(),
            display_name: "Test".into(),
            url: "https://exchange.example.invalid/EWS/Exchange.asmx".into(),
            username: "test@example.invalid".into(),
            auth: "password".into(),
        }
    }
    #[test]
    fn ews_requires_https_and_does_not_accept_embedded_or_redirect_credentials() {
        let mut s = settings();
        let c = Credentials {
            password: "synthetic".into(),
        };
        assert!(validate(&s, &c).is_ok());
        for u in [
            "http://exchange.invalid/EWS/Exchange.asmx",
            "https://user:secret@exchange.invalid/EWS/Exchange.asmx",
            "https://exchange.invalid/EWS/Exchange.asmx?token=secret",
        ] {
            s.url = u.into();
            assert!(validate(&s, &c).is_err());
        }
    }
    #[test]
    fn xml_values_cannot_escape_requests_or_enable_entities() {
        let text = "\"><evil>&'";
        let request = envelope(&format!(
            "<m:FindItem><m:QueryString>{}</m:QueryString></m:FindItem>",
            xml(text)
        ));
        let d = parse(&request).unwrap();
        assert_eq!(
            d.descendants()
                .find(|n| n.is_element() && n.tag_name().name() == "QueryString")
                .unwrap()
                .text(),
            Some(text)
        );
        assert!(parse("<!DOCTYPE x [<!ENTITY s SYSTEM 'file:///private'>]><x>&s;</x>").is_err());
    }
    #[test]
    fn ews_message_keeps_html_separate_and_conversation_ids() {
        let d=parse("<Message><ItemId Id='m' ChangeKey='c'/><Subject>Test</Subject><Body BodyType='HTML'>&lt;p&gt;Hello&lt;/p&gt;</Body><TextBody>Hello</TextBody><From><Mailbox><Name>Sender</Name><EmailAddress>a@example.invalid</EmailAddress></Mailbox></From><IsRead>true</IsRead><ConversationId Id='thread'/><InternetMessageId>&lt;id@example.invalid&gt;</InternetMessageId><Attachments><FileAttachment><AttachmentId Id='file'/><Name>report.txt</Name><Size>3</Size></FileAttachment></Attachments></Message>").unwrap();
        let v = message(d.root_element());
        assert_eq!(v["body"], "Hello");
        assert_eq!(v["html"], "<p>Hello</p>");
        assert_eq!(v["threadId"], "thread");
        assert_eq!(v["attachments"][0]["id"], "file");
        assert_eq!(v["unread"], false);
    }
    #[test]
    fn errors_are_not_reported_as_success_or_echo_server_messages() {
        let raw=envelope("<Response><ResponseCode>ErrorAccessDenied</ResponseCode><MessageText>private server data</MessageText></Response>");
        let d = parse(&raw).unwrap();
        let e = check_response(&d).unwrap_err();
        assert!(e.contains("ErrorAccessDenied"));
        assert!(!e.contains("private server data"));
        assert!(check_response(&parse("<html><body>Success</body></html>").unwrap()).is_err());
        assert!(check_response(&parse(&envelope("<Response/> ")).unwrap()).is_err());
    }
}
