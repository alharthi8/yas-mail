use super::*;

const GOOGLE_FIELDS: &str =
    "id,name,mimeType,size,modifiedTime,webViewLink,capabilities(canDownload)";
const GRAPH_FIELDS: &str = "id,name,size,file,folder,package,lastModifiedDateTime,webUrl";

pub(super) async fn profile(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
) -> Result<(String, String, Option<String>, Option<String>)> {
    let (id, name, email, namespace) = match c.provider.as_str() {
        "google" => {
            let mut url = api_url("https://www.googleapis.com/drive/v3/", &["about"])?;
            url.query_pairs_mut()
                .append_pair("fields", "user(displayName,emailAddress,permissionId)");
            let v = get(state, c, s, url).await?;
            let user = &v["user"];
            (
                user["permissionId"]
                    .as_str()
                    .or(user["emailAddress"].as_str()),
                user["displayName"].as_str(),
                user["emailAddress"].as_str(),
                None,
            )
                .map_owned()
        }
        "onedrive" => {
            let mut url = api_url("https://graph.microsoft.com/v1.0/", &["me"])?;
            url.query_pairs_mut()
                .append_pair("$select", "id,displayName,mail,userPrincipalName");
            let v = get(state, c, s, url).await?;
            (
                v["id"].as_str(),
                v["displayName"].as_str(),
                v["mail"]
                    .as_str()
                    .filter(|v| !v.is_empty())
                    .or(v["userPrincipalName"].as_str()),
                None,
            )
                .map_owned()
        }
        "dropbox" => {
            let v = dropbox_post(state, c, s, "users/get_current_account", Value::Null).await?;
            (
                v["account_id"].as_str(),
                v["name"]["display_name"].as_str(),
                v["email"].as_str(),
                v["root_info"]["root_namespace_id"].as_str(),
            )
                .map_owned()
        }
        "zoho" => {
            let v = get(state, c, s, zoho_url(s, &["users", "me"])?).await?;
            let user = json_rows(&v["data"])
                .first()
                .copied()
                .ok_or("Zoho did not return the WorkDrive user profile")?;
            (
                user["id"].as_str(),
                user["attributes"]["display_name"].as_str(),
                user["attributes"]["email_id"].as_str(),
                None,
            )
                .map_owned()
        }
        _ => return Err("Unsupported cloud provider".into()),
    };
    let id = id
        .filter(|v| !v.is_empty())
        .ok_or("The provider did not return an account identifier")?;
    opaque_id(&id)?;
    let label = clean_text(
        name.as_deref()
            .or(email.as_deref())
            .unwrap_or(provider_name(&c.provider)?),
        120,
    );
    let email = email.map(|v| clean_text(&v, 254));
    Ok((id, label, email, namespace))
}
trait OwnedProfile {
    fn map_owned(
        self,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
}
impl OwnedProfile for (Option<&str>, Option<&str>, Option<&str>, Option<&str>) {
    fn map_owned(
        self,
    ) -> (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) {
        (
            self.0.map(String::from),
            self.1.map(String::from),
            self.2.map(String::from),
            self.3.map(String::from),
        )
    }
}
fn required_name(v: &Value) -> String {
    clean_text(
        v.as_str()
            .filter(|v| !v.is_empty())
            .unwrap_or("Untitled file"),
        255,
    )
    .replace(['/', '\\'], "_")
}
fn google_item(v: &Value) -> Option<Item> {
    let id = v["id"].as_str()?;
    if opaque_id(id).is_err() {
        return None;
    }
    let mime = v["mimeType"].as_str().unwrap_or("application/octet-stream");
    let folder = mime == "application/vnd.google-apps.folder";
    let native = mime.starts_with("application/vnd.google-apps.");
    Some(Item {
        id: id.into(),
        name: required_name(&v["name"]),
        is_folder: folder,
        mime_type: mime.into(),
        size: number(&v["size"]),
        modified_at: v["modifiedTime"].as_str().map(String::from),
        web_url: web_link("google", v["webViewLink"].as_str()),
        can_download: !folder
            && !native
            && v["capabilities"]["canDownload"].as_bool().unwrap_or(true),
    })
}
fn graph_item(v: &Value) -> Option<Item> {
    let id = v["id"].as_str()?;
    if opaque_id(id).is_err() {
        return None;
    }
    let folder = v.get("folder").is_some_and(|f| !f.is_null());
    let file = v.get("file").is_some_and(|f| !f.is_null());
    Some(Item {
        id: id.into(),
        name: required_name(&v["name"]),
        is_folder: folder,
        mime_type: v["file"]["mimeType"]
            .as_str()
            .unwrap_or(if folder {
                "application/x-directory"
            } else {
                "application/octet-stream"
            })
            .into(),
        size: number(&v["size"]),
        modified_at: v["lastModifiedDateTime"].as_str().map(String::from),
        web_url: web_link("onedrive", v["webUrl"].as_str()),
        can_download: file && !folder,
    })
}
fn dropbox_item(v: &Value) -> Option<Item> {
    let folder = v[".tag"] == "folder";
    if v[".tag"] != "file" && !folder {
        return None;
    }
    // Folder paths are accepted by search_v2's options.path; files retain the
    // provider's stable id: identifier for download and existing-link lookup.
    let id = if folder {
        v["path_lower"].as_str().or(v["path_display"].as_str())
    } else {
        v["id"].as_str()
    }?;
    if opaque_id(id).is_err() {
        return None;
    }
    Some(Item {
        id: id.into(),
        name: required_name(&v["name"]),
        is_folder: folder,
        mime_type: if folder {
            "application/x-directory"
        } else {
            "application/octet-stream"
        }
        .into(),
        size: number(&v["size"]),
        modified_at: v["server_modified"].as_str().map(String::from),
        web_url: None,
        can_download: !folder && v["is_downloadable"].as_bool().unwrap_or(true),
    })
}
#[derive(Serialize, Deserialize)]
struct ZohoNode {
    kind: String,
    id: String,
    team: String,
}
fn zoho_node(kind: &str, id: &str, team: &str) -> String {
    format!(
        "z:{}",
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&ZohoNode {
                kind: kind.into(),
                id: id.into(),
                team: team.into()
            })
            .expect("node serializes")
        )
    )
}
fn parse_zoho_node(value: &str) -> Result<ZohoNode> {
    let raw = value.strip_prefix("z:").ok_or("Invalid WorkDrive item")?;
    if raw.len() > 4096 {
        return Err("Invalid WorkDrive item".into());
    }
    let node: ZohoNode = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(raw)
            .map_err(|_| "Invalid WorkDrive item")?,
    )
    .map_err(|_| "Invalid WorkDrive item")?;
    opaque_id(&node.id)?;
    if !node.team.is_empty() {
        opaque_id(&node.team)?;
    }
    if !["team", "teamfolder", "private", "file"].contains(&node.kind.as_str()) {
        return Err("Invalid WorkDrive item type".into());
    }
    Ok(node)
}
fn zoho_item(v: &Value, kind: &str, team: &str) -> Option<Item> {
    let id = v["id"].as_str()?;
    if opaque_id(id).is_err() {
        return None;
    }
    let a = &v["attributes"];
    let folder =
        kind == "teamfolder" || a["is_folder"].as_bool().unwrap_or(false) || a["type"] == "folder";
    let native = a["service_type"]
        .as_str()
        .is_some_and(|s| !s.is_empty() && s != "upload");
    let modified = number(&a["modified_time_in_millisecond"])
        .and_then(|m| chrono::DateTime::from_timestamp_millis(m as i64))
        .map(|d| d.to_rfc3339());
    Some(Item {
        id: zoho_node(kind, id, team),
        name: required_name(&a["name"]),
        is_folder: folder,
        mime_type: a["mime_type"]
            .as_str()
            .unwrap_or(if folder {
                "application/x-directory"
            } else {
                "application/octet-stream"
            })
            .into(),
        size: number(&a["storage_info"]["size_in_bytes"]),
        modified_at: modified,
        web_url: web_link("zoho", a["permalink"].as_str()),
        can_download: !folder
            && !native
            && a["capabilities"]["can_download"].as_bool().unwrap_or(true),
    })
}
fn sort_items(items: &mut [Item]) {
    items.sort_by(|a, b| {
        b.is_folder
            .cmp(&a.is_folder)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}
fn google_query(folder: Option<&str>, query: &str) -> String {
    let escaped = |v: &str| v.replace('\\', "\\\\").replace('\'', "\\'");
    let mut q = "trashed = false".to_string();
    if query.is_empty() {
        q.push_str(&format!(
            " and '{}' in parents",
            escaped(folder.unwrap_or("root"))
        ));
    } else {
        q.push_str(&format!(" and name contains '{}'", escaped(query)));
        if let Some(folder) = folder {
            q.push_str(&format!(" and '{}' in parents", escaped(folder)));
        }
    }
    q
}
fn graph_next(value: &str) -> Result<Url> {
    let url = Url::parse(value).map_err(|_| "Invalid OneDrive page cursor")?;
    if !valid_https(&url)
        || url.host_str() != Some("graph.microsoft.com")
        || !url.path().starts_with("/v1.0/me/drive/")
    {
        return Err("Unsupported OneDrive page cursor".into());
    }
    Ok(url)
}
fn zoho_url(s: &Secret, segments: &[&str]) -> Result<Url> {
    api_url(
        &format!("https://{}/workdrive/api/v1/", zoho_domains(&s.region)?.1),
        segments,
    )
}
fn offset_cursor(cursor: Option<&str>) -> Result<usize> {
    match cursor {
        None => Ok(0),
        Some(v) => {
            let n = v
                .parse::<usize>()
                .map_err(|_| "Invalid WorkDrive page cursor")?;
            if n > 10_000_000 {
                return Err("WorkDrive page cursor is too large".into());
            }
            Ok(n)
        }
    }
}

pub(super) async fn list(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
    folder: Option<&str>,
    query: &str,
    cursor: Option<&str>,
) -> Result<Page> {
    match c.provider.as_str() {
        "google" => {
            let mut url = api_url("https://www.googleapis.com/drive/v3/", &["files"])?;
            url.query_pairs_mut()
                .append_pair("q", &google_query(folder, query))
                .append_pair("fields", &format!("nextPageToken,files({GOOGLE_FIELDS})"))
                .append_pair("pageSize", "100")
                .append_pair("orderBy", "folder,name")
                .append_pair("supportsAllDrives", "true")
                .append_pair("includeItemsFromAllDrives", "true");
            if let Some(cursor) = cursor {
                url.query_pairs_mut().append_pair("pageToken", cursor);
            }
            let v = get(state, c, s, url).await?;
            let mut items = v["files"]
                .as_array()
                .map(|rows| rows.iter().filter_map(google_item).collect::<Vec<_>>())
                .unwrap_or_default();
            sort_items(&mut items);
            Ok(Page {
                items,
                cursor: v["nextPageToken"].as_str().map(String::from),
            })
        }
        "onedrive" => {
            let url = if let Some(cursor) = cursor {
                graph_next(cursor)?
            } else {
                let mut segments = vec!["me", "drive"];
                let search;
                if let Some(folder) = folder {
                    segments.extend(["items", folder]);
                } else {
                    segments.push("root");
                }
                if query.is_empty() {
                    segments.push("children");
                } else {
                    search = format!("search(q='{}')", query.replace('\'', "''"));
                    segments.push(&search);
                }
                let mut url = api_url("https://graph.microsoft.com/v1.0/", &segments)?;
                url.query_pairs_mut()
                    .append_pair("$select", GRAPH_FIELDS)
                    .append_pair("$top", "100");
                url
            };
            let v = get(state, c, s, url).await?;
            let mut items = v["value"]
                .as_array()
                .map(|rows| rows.iter().filter_map(graph_item).collect::<Vec<_>>())
                .unwrap_or_default();
            sort_items(&mut items);
            let next = v["@odata.nextLink"]
                .as_str()
                .map(|v| graph_next(v).map(|u| u.to_string()))
                .transpose()?;
            Ok(Page {
                items,
                cursor: next,
            })
        }
        "dropbox" => {
            let path = folder.unwrap_or("");
            let v = if query.is_empty() {
                if let Some(cursor) = cursor {
                    dropbox_post(
                        state,
                        c,
                        s,
                        "files/list_folder/continue",
                        json!({"cursor":cursor}),
                    )
                    .await?
                } else {
                    dropbox_post(state,c,s,"files/list_folder",json!({"path":path,"limit":100,"include_deleted":false,"include_non_downloadable_files":true})).await?
                }
            } else if let Some(cursor) = cursor {
                dropbox_post(
                    state,
                    c,
                    s,
                    "files/search/continue_v2",
                    json!({"cursor":cursor}),
                )
                .await?
            } else {
                dropbox_post(state,c,s,"files/search_v2",json!({"query":query,"options":{"path":path,"max_results":100,"filename_only":true}})).await?
            };
            let mut items = if query.is_empty() {
                v["entries"]
                    .as_array()
                    .map(|rows| rows.iter().filter_map(dropbox_item).collect::<Vec<_>>())
                    .unwrap_or_default()
            } else {
                v["matches"]
                    .as_array()
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|row| {
                                let m = &row["metadata"];
                                dropbox_item(m.get("metadata").unwrap_or(m))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            };
            sort_items(&mut items);
            Ok(Page {
                items,
                cursor: if v["has_more"].as_bool().unwrap_or(false) {
                    v["cursor"].as_str().map(String::from)
                } else {
                    None
                },
            })
        }
        "zoho" => zoho_list(state, c, s, folder, query, cursor).await,
        _ => Err("Unsupported cloud provider".into()),
    }
}

async fn zoho_list(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
    folder: Option<&str>,
    query: &str,
    cursor: Option<&str>,
) -> Result<Page> {
    let offset = offset_cursor(cursor)?;
    let Some(folder) = folder else {
        if !query.is_empty() {
            return Err("Open a WorkDrive team first, then search its files.".into());
        }
        let mut url = zoho_url(s, &["users", &s.user_id, "teams"])?;
        url.query_pairs_mut()
            .append_pair("page[limit]", "50")
            .append_pair("page[offset]", &offset.to_string());
        let v = get(state, c, s, url).await?;
        let rows = json_rows(&v["data"]);
        let items = rows
            .iter()
            .filter_map(|team| {
                let id = team["id"].as_str()?;
                Some(Item {
                    id: zoho_node("team", id, id),
                    name: required_name(&team["attributes"]["name"]),
                    is_folder: true,
                    mime_type: "application/x-directory".into(),
                    size: None,
                    modified_at: None,
                    web_url: None,
                    can_download: false,
                })
            })
            .collect();
        return Ok(Page {
            items,
            cursor: (rows.len() == 50).then(|| (offset + 50).to_string()),
        });
    };
    let mut node = parse_zoho_node(folder)?;
    if node.kind == "private" {
        let member = get(
            state,
            c,
            s,
            zoho_url(s, &["teams", &node.team, "currentuser"])?,
        )
        .await?;
        let members = json_rows(&member["data"]);
        let member = members
            .first()
            .and_then(|v| v["id"].as_str())
            .ok_or("WorkDrive did not return the team membership")?;
        let space = get(
            state,
            c,
            s,
            zoho_url(s, &["users", member, "privatespace"])?,
        )
        .await?;
        let spaces = json_rows(&space["data"]);
        let id = spaces
            .first()
            .and_then(|v| v["id"].as_str())
            .ok_or("My Folders is unavailable in this WorkDrive team")?;
        node.id = id.into();
    }
    let mut url = if !query.is_empty() {
        let mut url = zoho_url(s, &["teams", &node.team, "records"])?;
        url.query_pairs_mut().append_pair("search[all]", query);
        match node.kind.as_str() {
            "private" => {
                url.query_pairs_mut()
                    .append_pair("filter[user]", &s.user_id);
            }
            "teamfolder" => {
                url.query_pairs_mut()
                    .append_pair("filter[teamFolder]", &node.id);
            }
            "file" => {
                url.query_pairs_mut()
                    .append_pair("filter[parentId]", &node.id);
            }
            _ => {}
        }
        url
    } else {
        match node.kind.as_str() {
            "team" => zoho_url(s, &["teams", &node.id, "teamfolders"])?,
            "teamfolder" => zoho_url(s, &["teamfolders", &node.id, "files"])?,
            "private" => zoho_url(s, &["privatespace", &node.id, "files"])?,
            "file" => zoho_url(s, &["files", &node.id, "files"])?,
            _ => return Err("Invalid WorkDrive folder".into()),
        }
    };
    url.query_pairs_mut()
        .append_pair("page[limit]", "50")
        .append_pair("page[offset]", &offset.to_string());
    let v = get(state, c, s, url).await?;
    let rows = json_rows(&v["data"]);
    let kind = if node.kind == "team" && query.is_empty() {
        "teamfolder"
    } else {
        "file"
    };
    let mut items = rows
        .iter()
        .filter_map(|v| zoho_item(v, kind, &node.team))
        .collect::<Vec<_>>();
    if node.kind == "team" && query.is_empty() && offset == 0 {
        items.insert(
            0,
            Item {
                id: zoho_node("private", &s.user_id, &node.team),
                name: "My Folders".into(),
                is_folder: true,
                mime_type: "application/x-directory".into(),
                size: None,
                modified_at: None,
                web_url: None,
                can_download: false,
            },
        );
    }
    sort_items(&mut items);
    Ok(Page {
        items,
        cursor: (rows.len() == 50).then(|| (offset + 50).to_string()),
    })
}

pub(super) async fn metadata(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
    id: &str,
    include_link: bool,
) -> Result<Item> {
    let item = match c.provider.as_str() {
        "google" => {
            let mut url = api_url("https://www.googleapis.com/drive/v3/", &["files", id])?;
            url.query_pairs_mut()
                .append_pair("fields", GOOGLE_FIELDS)
                .append_pair("supportsAllDrives", "true");
            google_item(&get(state, c, s, url).await?)
        }
        "onedrive" => {
            let mut url = api_url(
                "https://graph.microsoft.com/v1.0/",
                &["me", "drive", "items", id],
            )?;
            url.query_pairs_mut().append_pair("$select", GRAPH_FIELDS);
            graph_item(&get(state, c, s, url).await?)
        }
        "dropbox" => {
            let v = dropbox_post(state, c, s, "files/get_metadata", json!({"path":id})).await?;
            let mut item = dropbox_item(&v);
            if include_link {
                if let Some(item) = &mut item {
                    let links = dropbox_post(
                        state,
                        c,
                        s,
                        "sharing/list_shared_links",
                        json!({"path":id,"direct_only":true}),
                    )
                    .await?;
                    item.web_url = links["links"].as_array().and_then(|rows| {
                        rows.iter()
                            .find_map(|row| web_link("dropbox", row["url"].as_str()))
                    });
                }
            }
            item
        }
        "zoho" => {
            let node = parse_zoho_node(id)?;
            if node.kind != "file" {
                return Err("Choose a WorkDrive file".into());
            }
            let v = get(state, c, s, zoho_url(s, &["files", &node.id])?).await?;
            json_rows(&v["data"])
                .first()
                .and_then(|v| zoho_item(v, "file", &node.team))
        }
        _ => return Err("Unsupported cloud provider".into()),
    };
    item.ok_or_else(|| {
        "The provider did not return that file. Refresh the folder and choose it again.".into()
    })
}
pub(super) fn download_request(
    state: &CloudState,
    c: &Connection,
    s: &Secret,
    id: &str,
) -> Result<reqwest::RequestBuilder> {
    let mut request = match c.provider.as_str() {
        "google" => {
            let mut url = api_url("https://www.googleapis.com/drive/v3/", &["files", id])?;
            url.query_pairs_mut()
                .append_pair("alt", "media")
                .append_pair("supportsAllDrives", "true");
            api_request(state, c, s, reqwest::Method::GET, url)
        }
        "onedrive" => api_request(
            state,
            c,
            s,
            reqwest::Method::GET,
            api_url(
                "https://graph.microsoft.com/v1.0/",
                &["me", "drive", "items", id, "content"],
            )?,
        ),
        "dropbox" => api_request(
            state,
            c,
            s,
            reqwest::Method::POST,
            Url::parse("https://content.dropboxapi.com/2/files/download").expect("constant URL"),
        )
        .header("Dropbox-API-Arg", json!({"path":id}).to_string()),
        "zoho" => {
            let node = parse_zoho_node(id)?;
            if node.kind != "file" {
                return Err("Choose a WorkDrive file".into());
            }
            api_request(
                state,
                c,
                s,
                reqwest::Method::GET,
                api_url(
                    &format!(
                        "https://{}/v1/workdrive/download/",
                        zoho_domains(&s.region)?.2
                    ),
                    &[&node.id],
                )?,
            )
        }
        _ => return Err("Unsupported cloud provider".into()),
    };
    request = request.header(reqwest::header::ACCEPT_ENCODING, "identity");
    Ok(request)
}
pub(super) fn allowed_download_redirect(provider: &str, url: &Url) -> bool {
    if !valid_https(url) {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let hosts: &[&str] = match provider {
        "google" => &["googleusercontent.com", "googleapis.com"],
        "onedrive" => &[
            "sharepoint.com",
            "sharepoint-df.com",
            "1drv.com",
            "onedrive.com",
            "storage.live.com",
        ],
        "dropbox" => &["dropboxusercontent.com", "dropboxapi.com", "dropbox.com"],
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
            "zohocdn.com",
            "zohoexternal.com",
        ],
        _ => return false,
    };
    hosts.iter().any(|base| host_matches(host, base))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn google_native_documents_and_restricted_files_are_link_only() {
        let doc=google_item(&json!({"id":"doc","name":"Notes","mimeType":"application/vnd.google-apps.document","webViewLink":"https://docs.google.com/document/d/doc/edit"})).unwrap();
        assert!(!doc.can_download);
        assert!(doc.web_url.is_some());
        let file=google_item(&json!({"id":"pdf","name":"Report.pdf","mimeType":"application/pdf","size":"123","capabilities":{"canDownload":false}})).unwrap();
        assert_eq!(file.size, Some(123));
        assert!(!file.can_download);
    }
    #[test]
    fn google_search_escapes_provider_query_syntax() {
        let q = google_query(Some("folder' or trashed=true"), "O'Reilly\\draft");
        assert!(q.contains("O\\'Reilly\\\\draft"));
        assert!(q.contains("folder\\' or trashed=true"));
        assert!(q.starts_with("trashed = false"));
    }
    #[test]
    fn onedrive_continuation_cannot_escape_graph_user_drive() {
        assert!(graph_next(
            "https://graph.microsoft.com/v1.0/me/drive/root/children?$skiptoken=abc"
        )
        .is_ok());
        for url in [
            "https://graph.microsoft.com.evil.invalid/v1.0/me/drive/root/children",
            "http://graph.microsoft.com/v1.0/me/drive/root/children",
            "https://graph.microsoft.com/v1.0/users/other/drive/root/children",
            "https://user@graph.microsoft.com/v1.0/me/drive/root/children",
        ] {
            assert!(graph_next(url).is_err());
        }
    }
    #[test]
    fn provider_metadata_preserves_download_restrictions_and_folder_semantics() {
        let folder = graph_item(&json!({"id":"a","name":"Folder","folder":{},"size":0})).unwrap();
        assert!(folder.is_folder);
        assert!(!folder.can_download);
        let file=dropbox_item(&json!({".tag":"file","id":"id:one","name":"Safe\r\nname.txt","size":2,"is_downloadable":false})).unwrap();
        assert!(!file.can_download);
        assert!(!file.name.contains('\r'));
        let folder = dropbox_item(
            &json!({".tag":"folder","id":"id:folder","path_lower":"/my folder","name":"My folder"}),
        )
        .unwrap();
        assert_eq!(folder.id, "/my folder");
    }
    #[test]
    fn workdrive_opaque_nodes_retain_team_context_without_urls() {
        let id = zoho_node("file", "folder/with?characters", "team1");
        let decoded = parse_zoho_node(&id).unwrap();
        assert_eq!(decoded.id, "folder/with?characters");
        assert_eq!(decoded.team, "team1");
        assert!(parse_zoho_node("https://evil.invalid/file").is_err());
    }
    #[test]
    fn workdrive_metadata_parses_bytes_and_native_link_only() {
        let v = json!({"id":"x","attributes":{"name":"Budget","service_type":"sheet","is_folder":false,"storage_info":{"size_in_bytes":12},"permalink":"https://sheet.zoho.eu/sheet/open/x"}});
        let file = zoho_item(&v, "file", "team").unwrap();
        assert_eq!(file.size, Some(12));
        assert!(!file.can_download);
        assert!(file.web_url.is_some());
    }
    #[test]
    fn download_redirects_are_https_provider_domains_only() {
        for url in [
            "http://tenant.sharepoint.com/file",
            "https://tenant.sharepoint.com.evil.invalid/file",
            "https://localhost/file",
            "https://127.0.0.1/file",
            "https://user:password@tenant.sharepoint.com/file",
        ] {
            assert!(!allowed_download_redirect(
                "onedrive",
                &Url::parse(url).unwrap()
            ));
        }
        assert!(allowed_download_redirect(
            "onedrive",
            &Url::parse("https://tenant.sharepoint.com/path?temporary=opaque").unwrap()
        ));
        assert!(!allowed_download_redirect(
            "google",
            &Url::parse("https://tenant.sharepoint.com/file").unwrap()
        ));
    }
}
