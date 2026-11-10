//! Rich contacts and calendar operations. All account commands enforce window ownership.
use crate::{err, google, segment, text, Account, App, Result};
use chrono::{DateTime, NaiveDate, Utc};
use serde_json::{json, Value};
use std::sync::Arc;

const PEOPLE: &str = "https://people.googleapis.com/v1";
const GRAPH: &str = "https://graph.microsoft.com/v1.0/me";
const CAL: &str = "https://www.googleapis.com/calendar/v3";
const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,metadata,organizations,biographies,birthdays,addresses,urls,memberships,photos";
const MAX_ROWS: usize = 20_000;

fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn field(value: &Value, name: &str) -> String {
    text(&value[name])
}
fn nonempty(value: &Value, name: &str) -> bool {
    !field(value, name).trim().is_empty()
}
fn graph_next(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).map_err(|_| "Invalid Microsoft continuation")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("graph.microsoft.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || !parsed.path().starts_with("/v1.0/me/")
    {
        return Err("Unexpected Microsoft continuation".into());
    }
    Ok(parsed.to_string())
}
fn bounded_text(value: &Value, name: &str, limit: usize) -> Result<String> {
    let s = field(value, name);
    if s.len() > limit || s.contains('\0') {
        return Err(format!("{name} exceeds the supported length"));
    }
    Ok(s)
}
fn person_resource(id: &str) -> Result<String> {
    let tail = id
        .strip_prefix("people/")
        .ok_or("Invalid Google contact identifier")?;
    if tail.is_empty() || tail.contains('/') || tail.len() > 512 {
        return Err("Invalid Google contact identifier".into());
    }
    Ok(format!("people/{}", segment(tail)))
}
fn google_contact(v: &Value) -> Value {
    let emails: Vec<_> = array(&v["emailAddresses"])
        .iter()
        .map(|x| json!({"value":x["value"],"label":x["type"]}))
        .collect();
    let phones: Vec<_> = array(&v["phoneNumbers"])
        .iter()
        .map(|x| json!({"value":x["value"],"label":x["type"]}))
        .collect();
    let date = &v["birthdays"][0]["date"];
    let birthday = match (
        date["year"].as_u64(),
        date["month"].as_u64(),
        date["day"].as_u64(),
    ) {
        (year, Some(m), Some(d)) => format!("{:04}-{m:02}-{d:02}", year.unwrap_or(1900)),
        _ => String::new(),
    };
    json!({"id":v["resourceName"],"etag":v["etag"],"metadata":v["metadata"],"name":v["names"][0]["displayName"],"givenName":v["names"][0]["givenName"],"familyName":v["names"][0]["familyName"],"email":emails.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"phone":phones.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"emails":emails,"phones":phones,"company":v["organizations"][0]["name"],"jobTitle":v["organizations"][0]["title"],"notes":v["biographies"][0]["value"],"birthday":birthday,"addresses":array(&v["addresses"]).iter().map(|x|json!({"street":x["streetAddress"],"city":x["city"],"region":x["region"],"postalCode":x["postalCode"],"country":x["country"],"label":x["type"]})).collect::<Vec<_>>(),"websites":array(&v["urls"]).iter().map(|x|json!({"value":x["value"],"label":x["type"]})).collect::<Vec<_>>(),"groups":array(&v["memberships"]).iter().filter_map(|x|x["contactGroupMembership"]["contactGroupResourceName"].as_str()).collect::<Vec<_>>(),"photoUrl":v["photos"][0]["url"]})
}
fn microsoft_contact(v: &Value, folder: Option<&str>) -> Value {
    let mut phones = vec![];
    if nonempty(v, "mobilePhone") {
        phones.push(json!({"value":v["mobilePhone"],"label":"mobile"}));
    }
    for (key, label) in [("businessPhones", "work"), ("homePhones", "home")] {
        for p in array(&v[key]) {
            phones.push(json!({"value":p,"label":label}));
        }
    }
    let emails: Vec<_> = array(&v["emailAddresses"])
        .iter()
        .map(|x| json!({"value":x["address"],"label":x["name"]}))
        .collect();
    let addresses:Vec<_>=[("homeAddress","home"),("businessAddress","work"),("otherAddress","other")].into_iter().filter(|(key,_)|v[*key].is_object()&&array(&v[*key]).is_empty()).filter_map(|(key,label)|{let x=&v[key];if ["street","city","state","postalCode","countryOrRegion"].iter().all(|k|field(x,k).is_empty()){None}else{Some(json!({"street":x["street"],"city":x["city"],"region":x["state"],"postalCode":x["postalCode"],"country":x["countryOrRegion"],"label":label}))}}).collect();
    json!({"id":v["id"],"etag":v["@odata.etag"],"name":v["displayName"],"givenName":v["givenName"],"familyName":v["surname"],"email":emails.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"phone":phones.first().map(|x|x["value"].clone()).unwrap_or(json!("")),"emails":emails,"phones":phones,"company":v["companyName"],"jobTitle":v["jobTitle"],"notes":v["personalNotes"],"birthday":v["birthday"].as_str().map(|s|s.chars().take(10).collect::<String>()).unwrap_or_default(),"addresses":addresses,"websites":if nonempty(v,"businessHomePage"){vec![json!({"value":v["businessHomePage"],"label":"work"})]}else{vec![]},"groups":folder.into_iter().collect::<Vec<_>>()})
}

pub async fn contacts(state: &App, account: &Account) -> Result<Value> {
    if let Some(settings) = account.exchange.clone() {
        return crate::exchange::contacts(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", account.id))?,
        )
        .await;
    }
    crate::require_groupware(account, "Contacts")?;
    let mut rows = vec![];
    if google(account) {
        let mut token = String::new();
        for _ in 0..100 {
            let mut url =
                url::Url::parse(&format!("{PEOPLE}/people/me/connections")).map_err(err)?;
            url.query_pairs_mut().extend_pairs([
                ("personFields", PERSON_FIELDS),
                ("pageSize", "1000"),
                ("pageToken", token.as_str()),
            ]);
            let value = state.api(account, "GET", url.as_str(), Value::Null).await?;
            rows.extend(array(&value["connections"]).iter().map(google_contact));
            if rows.len() > MAX_ROWS {
                return Err("Contacts exceed the 20,000-record limit".into());
            }
            token = field(&value, "nextPageToken");
            if token.is_empty() {
                break;
            }
        }
    } else {
        let folders = groups(state, account).await?;
        let mut seen = std::collections::HashSet::new();
        for folder in std::iter::once(None).chain(
            array(&folders)
                .iter()
                .filter_map(|x| x["id"].as_str())
                .map(Some),
        ) {
            let mut next = if let Some(id) = folder {
                format!("{GRAPH}/contactFolders/{}/contacts?$top=100", segment(id))
            } else {
                format!("{GRAPH}/contacts?$top=100")
            };
            for _ in 0..200 {
                let value = state.api(account, "GET", &next, Value::Null).await?;
                for item in array(&value["value"]) {
                    if let Some(id) = item["id"].as_str() {
                        if seen.insert(id.to_owned()) {
                            rows.push(microsoft_contact(item, folder));
                        }
                    }
                }
                if rows.len() > MAX_ROWS {
                    return Err("Contacts exceed the 20,000-record limit".into());
                }
                let url = field(&value, "@odata.nextLink");
                if url.is_empty() {
                    break;
                }
                next = graph_next(&url)?;
            }
        }
    }
    Ok(json!(rows))
}
fn normalized_values(contact: &Value, key: &str, fallback: &str) -> Vec<Value> {
    if let Some(values) = contact[key].as_array() {
        values
            .iter()
            .filter(|x| nonempty(x, "value"))
            .cloned()
            .collect()
    } else if nonempty(contact, fallback) {
        vec![json!({"value":contact[fallback],"label":"other"})]
    } else {
        vec![]
    }
}
fn validate_contact(c: &Value) -> Result<()> {
    if bounded_text(c, "name", 512)?.trim().is_empty() {
        return Err("Enter a contact name".into());
    }
    for key in ["givenName", "familyName", "company", "jobTitle"] {
        bounded_text(c, key, 512)?;
    }
    bounded_text(c, "notes", 30_000)?;
    for key in ["emails", "phones", "addresses", "websites", "groups"] {
        if array(&c[key]).len() > 100 {
            return Err(format!("Too many {key}"));
        }
    }
    for email in normalized_values(c, "emails", "email") {
        crate::addresses(&field(&email, "value"))?;
    }
    if nonempty(c, "birthday") {
        NaiveDate::parse_from_str(&field(c, "birthday"), "%Y-%m-%d")
            .map_err(|_| "Birthday must be YYYY-MM-DD")?;
    }
    Ok(())
}
pub async fn save_contact(state: &App, a: &Account, contact: Value, delete: bool) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::save_contact(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            contact,
            delete,
        )
        .await;
    }
    crate::require_groupware(a, "Contacts")?;
    let id = field(&contact, "id");
    if delete {
        if id.is_empty() {
            return Err("Choose a saved contact to delete".into());
        }
        return state
            .api(
                a,
                "DELETE",
                &if google(a) {
                    format!("{PEOPLE}/{}:deleteContact", person_resource(&id)?)
                } else {
                    format!("{GRAPH}/contacts/{}", segment(&id))
                },
                Value::Null,
            )
            .await;
    }
    validate_contact(&contact)?;
    let emails = normalized_values(&contact, "emails", "email");
    let phones = normalized_values(&contact, "phones", "phone");
    if google(a) {
        let mut body = json!({"names":[{"givenName":if nonempty(&contact,"givenName"){contact["givenName"].clone()}else{contact["name"].clone()},"familyName":contact["familyName"].as_str().unwrap_or("")}],"emailAddresses":emails.iter().map(|x|json!({"value":x["value"],"type":x["label"].as_str().unwrap_or("other")})).collect::<Vec<_>>(),"phoneNumbers":phones.iter().map(|x|json!({"value":x["value"],"type":x["label"].as_str().unwrap_or("other")})).collect::<Vec<_>>()});
        let mut fields = vec!["names", "emailAddresses", "phoneNumbers"];
        for (source, target, value) in [
            (
                "company",
                "organizations",
                json!([{"name":contact["company"].as_str().unwrap_or(""),"title":contact["jobTitle"].as_str().unwrap_or("")} ]),
            ),
            (
                "notes",
                "biographies",
                json!([{"value":contact["notes"].as_str().unwrap_or(""),"contentType":"TEXT_PLAIN"}]),
            ),
        ] {
            if contact.get(source).is_some() {
                body[target] = value;
                fields.push(target);
            }
        }
        if contact.get("addresses").is_some() {
            body["addresses"]=json!(array(&contact["addresses"]).iter().map(|x|json!({"streetAddress":x["street"],"city":x["city"],"region":x["region"],"postalCode":x["postalCode"],"country":x["country"],"type":x["label"]})).collect::<Vec<_>>());
            fields.push("addresses");
        }
        if contact.get("websites").is_some() {
            body["urls"] = json!(array(&contact["websites"])
                .iter()
                .map(|x| json!({"value":x["value"],"type":x["label"]}))
                .collect::<Vec<_>>());
            fields.push("urls");
        }
        if contact.get("birthday").is_some() {
            let birthday = field(&contact, "birthday");
            body["birthdays"] = if birthday.is_empty() {
                json!([])
            } else {
                let d = NaiveDate::parse_from_str(&birthday, "%Y-%m-%d").map_err(err)?;
                use chrono::Datelike;
                json!([{"date":{"year":d.year(),"month":d.month(),"day":d.day()}}])
            };
            fields.push("birthdays");
        }
        let url = if id.is_empty() {
            format!("{PEOPLE}/people:createContact?personFields={PERSON_FIELDS}")
        } else {
            let resource = person_resource(&id)?;
            let current = state
                .api(
                    a,
                    "GET",
                    &format!("{PEOPLE}/{resource}?personFields={PERSON_FIELDS}"),
                    Value::Null,
                )
                .await?;
            if nonempty(&contact, "etag") && current["etag"] != contact["etag"] {
                return Err("This contact changed at Google. Refresh before saving.".into());
            }
            body["etag"] = current["etag"].clone();
            body["metadata"] = current["metadata"].clone();
            format!("{PEOPLE}/{resource}:updateContact?updatePersonFields={}&personFields={PERSON_FIELDS}",fields.join(","))
        };
        let result = state
            .api(a, if id.is_empty() { "POST" } else { "PATCH" }, &url, body)
            .await?;
        if let Some(groups) = contact["groups"].as_array() {
            let resource = field(&result, "resourceName");
            for group in groups.iter().filter_map(Value::as_str) {
                if group.starts_with("contactGroups/") {
                    state
                        .api(
                            a,
                            "POST",
                            &format!("{PEOPLE}/{}/members:modify", group_resource(group)?),
                            json!({"resourceNamesToAdd":[resource]}),
                        )
                        .await?;
                }
            }
        }
        Ok(google_contact(&result))
    } else {
        let url = if id.is_empty() {
            if let Some(folder) = array(&contact["groups"]).first().and_then(Value::as_str) {
                format!("{GRAPH}/contactFolders/{}/contacts", segment(folder))
            } else {
                format!("{GRAPH}/contacts")
            }
        } else {
            format!("{GRAPH}/contacts/{}", segment(&id))
        };
        let mut body = json!({"givenName":if nonempty(&contact,"givenName"){contact["givenName"].clone()}else{contact["name"].clone()},"surname":contact["familyName"].as_str().unwrap_or(""),"displayName":contact["name"],"emailAddresses":emails.iter().map(|x|json!({"address":x["value"],"name":x["label"].as_str().unwrap_or("")})).collect::<Vec<_>>(),"mobilePhone":phones.iter().find(|x|x["label"]=="mobile").or_else(||phones.first()).map(|x|x["value"].clone()).unwrap_or(json!("")),"homePhones":phones.iter().filter(|x|x["label"]=="home").map(|x|x["value"].clone()).collect::<Vec<_>>(),"businessPhones":phones.iter().filter(|x|x["label"]=="work").map(|x|x["value"].clone()).collect::<Vec<_>>()});
        for (from, to) in [
            ("company", "companyName"),
            ("jobTitle", "jobTitle"),
            ("notes", "personalNotes"),
        ] {
            if let Some(value) = contact.get(from) {
                body[to] = value.clone();
            }
        }
        if contact.get("birthday").is_some() {
            let birthday = field(&contact, "birthday");
            body["birthday"] = if birthday.is_empty() {
                Value::Null
            } else {
                json!(format!("{birthday}T00:00:00Z"))
            };
        }
        if let Some(addresses) = contact["addresses"].as_array() {
            for (label, key) in [
                ("home", "homeAddress"),
                ("work", "businessAddress"),
                ("other", "otherAddress"),
            ] {
                let x = addresses
                    .iter()
                    .find(|x| x["label"] == label)
                    .cloned()
                    .unwrap_or(json!({}));
                body[key] = json!({"street":x["street"].as_str().unwrap_or(""),"city":x["city"].as_str().unwrap_or(""),"state":x["region"].as_str().unwrap_or(""),"postalCode":x["postalCode"].as_str().unwrap_or(""),"countryOrRegion":x["country"].as_str().unwrap_or("")});
            }
        }
        if contact.get("websites").is_some() {
            body["businessHomePage"] = array(&contact["websites"])
                .first()
                .map(|x| x["value"].clone())
                .unwrap_or(json!(""));
        }
        let value = state
            .api(a, if id.is_empty() { "POST" } else { "PATCH" }, &url, body)
            .await?;
        Ok(microsoft_contact(
            &value,
            array(&contact["groups"]).first().and_then(Value::as_str),
        ))
    }
}
fn group_resource(id: &str) -> Result<String> {
    let tail = id
        .strip_prefix("contactGroups/")
        .ok_or("Invalid contact group")?;
    if tail.is_empty() || tail.contains('/') {
        return Err("Invalid contact group".into());
    }
    Ok(format!("contactGroups/{}", segment(tail)))
}
pub async fn groups(state: &App, a: &Account) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::contact_groups(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
        )
        .await;
    }
    crate::require_groupware(a, "Contacts")?;
    let mut rows = vec![];
    let mut next = if google(a) {
        format!("{PEOPLE}/contactGroups?pageSize=1000")
    } else {
        format!("{GRAPH}/contactFolders?$top=100")
    };
    for _ in 0..100 {
        let v = state.api(a, "GET", &next, Value::Null).await?;
        for r in array(if google(a) {
            &v["contactGroups"]
        } else {
            &v["value"]
        }) {
            rows.push(json!({"id":if google(a){&r["resourceName"]}else{&r["id"]},"name":if google(a){&r["name"]}else{&r["displayName"]},"kind":if google(a){"group"}else{"folder"},"editable":!google(a)||r["groupType"]=="USER_CONTACT_GROUP"}));
        }
        let token = field(
            &v,
            if google(a) {
                "nextPageToken"
            } else {
                "@odata.nextLink"
            },
        );
        if token.is_empty() {
            break;
        }
        next = if google(a) {
            format!(
                "{PEOPLE}/contactGroups?pageSize=1000&pageToken={}",
                segment(&token)
            )
        } else {
            graph_next(&token)?
        };
    }
    Ok(json!(rows))
}

async fn owned(window: &tauri::WebviewWindow, state: &Arc<App>, id: &str) -> Result<Account> {
    crate::windows::check_owner(state, window.label(), id)?;
    let a = state.account(id).await?;
    crate::require_groupware(&a, "Contacts/calendar")?;
    Ok(a)
}
#[tauri::command]
pub async fn contact_groups(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
) -> Result<Value> {
    let a = owned(&window, &state, &account).await?;
    groups(&state, &a).await
}
#[tauri::command]
pub async fn contact_group_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    group: Value,
    delete: bool,
) -> Result<Value> {
    let a = owned(&window, &state, &account).await?;
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::contact_group_save(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            group,
            delete,
        )
        .await;
    }
    let id = field(&group, "id");
    let name = bounded_text(&group, "name", 256)?;
    if !delete && name.trim().is_empty() {
        return Err("Enter a group/folder name".into());
    }
    if delete && id.is_empty() {
        return Err("Choose a saved contact group".into());
    }
    if google(&a) {
        let resource = if id.is_empty() {
            String::new()
        } else {
            group_resource(&id)?
        };
        let url = if id.is_empty() {
            format!("{PEOPLE}/contactGroups")
        } else {
            format!("{PEOPLE}/{resource}")
        };
        let value=state.api(&a,if delete{"DELETE"}else if id.is_empty(){"POST"}else{"PUT"},&url,if delete{Value::Null}else{json!({"contactGroup":{"resourceName":if id.is_empty(){Value::Null}else{json!(id)},"name":name}})}).await?;
        if let Some(members) = group["members"].as_array() {
            if members.len() > 1000 {
                return Err("A group update supports up to 1000 members".into());
            }
            let id = if id.is_empty() {
                field(&value, "resourceName")
            } else {
                id
            };
            let members = members
                .iter()
                .map(|x| person_resource(x.as_str().unwrap_or("")))
                .collect::<Result<Vec<_>>>()?;
            state
                .api(
                    &a,
                    "POST",
                    &format!("{PEOPLE}/{}/members:modify", group_resource(&id)?),
                    json!({"resourceNamesToAdd":members}),
                )
                .await?;
        }
        Ok(value)
    } else {
        let url = if id.is_empty() {
            format!("{GRAPH}/contactFolders")
        } else {
            format!("{GRAPH}/contactFolders/{}", segment(&id))
        };
        state
            .api(
                &a,
                if delete {
                    "DELETE"
                } else if id.is_empty() {
                    "POST"
                } else {
                    "PATCH"
                },
                &url,
                if delete {
                    Value::Null
                } else {
                    json!({"displayName":name})
                },
            )
            .await
    }
}

fn calendar_id(calendar: Option<&str>) -> String {
    calendar
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("primary")
        .into()
}
fn event_base(a: &Account, calendar: &str) -> String {
    if google(a) {
        format!("{CAL}/calendars/{}/events", segment(calendar))
    } else if calendar == "primary" {
        format!("{GRAPH}/events")
    } else {
        format!("{GRAPH}/calendars/{}/events", segment(calendar))
    }
}
pub async fn calendars(state: &App, a: &Account) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::calendars(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
        )
        .await;
    }
    crate::require_groupware(a, "Calendar")?;
    let mut rows = vec![];
    let mut next = if google(a) {
        format!("{CAL}/users/me/calendarList?maxResults=250")
    } else {
        format!("{GRAPH}/calendars?$top=100")
    };
    for _ in 0..100 {
        let v = state.api(a, "GET", &next, Value::Null).await?;
        for x in array(if google(a) { &v["items"] } else { &v["value"] }) {
            rows.push(json!({"id":x["id"],"name":if google(a){x["summary"].clone()}else{x["name"].clone()},"color":if google(a){x["backgroundColor"].clone()}else{x["hexColor"].clone()},"primary":if google(a){x["primary"].as_bool().unwrap_or(false)}else{x["isDefaultCalendar"].as_bool().unwrap_or(false)},"canEdit":if google(a){["owner","writer"].contains(&x["accessRole"].as_str().unwrap_or(""))}else{x["canEdit"].as_bool().unwrap_or(true)},"allowedOnlineMeetingProviders":x["allowedOnlineMeetingProviders"]}));
        }
        if rows.len() > 2000 {
            return Err("Too many calendars".into());
        }
        let token = field(
            &v,
            if google(a) {
                "nextPageToken"
            } else {
                "@odata.nextLink"
            },
        );
        if token.is_empty() {
            break;
        }
        next = if google(a) {
            format!(
                "{CAL}/users/me/calendarList?maxResults=250&pageToken={}",
                segment(&token)
            )
        } else {
            graph_next(&token)?
        };
    }
    Ok(json!(rows))
}
fn event_date(v: &Value, g: bool) -> String {
    if g {
        v["dateTime"]
            .as_str()
            .or(v["date"].as_str())
            .unwrap_or("")
            .into()
    } else {
        let s = field(v, "dateTime");
        if s.is_empty() {
            s
        } else if s.ends_with('Z') || DateTime::parse_from_rfc3339(&s).is_ok() {
            s
        } else if v["timeZone"] == "UTC" {
            format!("{s}Z")
        } else {
            s
        }
    }
}
fn normalize_event(v: &Value, g: bool, calendar: &str) -> Value {
    let attendees:Vec<_>=array(&v["attendees"]).iter().map(|x|if g{json!({"email":x["email"],"name":x["displayName"],"response":x["responseStatus"],"self":x["self"]})}else{json!({"email":x["emailAddress"]["address"],"name":x["emailAddress"]["name"],"response":x["status"]["response"]})}).collect();
    let reminders = if g {
        array(&v["reminders"]["overrides"])
            .iter()
            .filter_map(|x| x["minutes"].as_u64())
            .collect::<Vec<_>>()
    } else if v["isReminderOn"] == true {
        vec![v["reminderMinutesBeforeStart"].as_u64().unwrap_or(15)]
    } else {
        vec![]
    };
    let link = if g {
        v["hangoutLink"]
            .as_str()
            .or_else(|| {
                array(&v["conferenceData"]["entryPoints"])
                    .iter()
                    .find(|x| x["entryPointType"] == "video")
                    .and_then(|x| x["uri"].as_str())
            })
            .unwrap_or("")
    } else {
        v["onlineMeeting"]["joinUrl"].as_str().unwrap_or("")
    };
    json!({"id":v["id"],"etag":if g{&v["etag"]}else{&v["@odata.etag"]},"calendarId":calendar,"title":if g{&v["summary"]}else{&v["subject"]},"start":event_date(&v["start"],g),"end":event_date(&v["end"],g),"allDay":if g{v["start"]["date"].is_string()}else{v["isAllDay"].as_bool().unwrap_or(false)},"timezone":if g{&v["start"]["timeZone"]}else{&v["start"]["timeZone"]},"description":if g{&v["description"]}else{&v["body"]["content"]},"location":if g{&v["location"]}else{&v["location"]["displayName"]},"attendees":attendees,"organizer":if g{v["organizer"]["email"].clone()}else{v["organizer"]["emailAddress"]["address"].clone()},"recurrence":v["recurrence"],"recurringEventId":if g{&v["recurringEventId"]}else{&v["seriesMasterId"]},"reminders":reminders,"onlineMeeting":!link.is_empty(),"link":link,"responseStatus":if g{array(&v["attendees"]).iter().find(|x|x["self"]==true).map(|x|x["responseStatus"].clone()).unwrap_or(Value::Null)}else{v["responseStatus"]["response"].clone()},"canEdit":if g{v["locked"]!=true}else{!v["isCancelled"].as_bool().unwrap_or(false)},"status":v["status"]})
}
pub async fn events(
    state: &App,
    a: &Account,
    start: String,
    end: String,
    calendar: Option<String>,
) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::events(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            start,
            end,
            calendar,
        )
        .await;
    }
    crate::require_groupware(a, "Calendar")?;
    let s = DateTime::parse_from_rfc3339(&start).map_err(|_| "Start time must be RFC3339")?;
    let e = DateTime::parse_from_rfc3339(&end).map_err(|_| "End time must be RFC3339")?;
    if e <= s || (e - s).num_days() > 730 {
        return Err("Choose a calendar period of up to two years".into());
    }
    let calendar = calendar_id(calendar.as_deref());
    let mut url = url::Url::parse(&if google(a) {
        event_base(a, &calendar)
    } else if calendar == "primary" {
        format!("{GRAPH}/calendarView")
    } else {
        format!("{GRAPH}/calendars/{}/calendarView", segment(&calendar))
    })
    .map_err(err)?;
    if google(a) {
        url.query_pairs_mut().extend_pairs([
            ("timeMin", start.as_str()),
            ("timeMax", end.as_str()),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", "250"),
        ]);
    } else {
        url.query_pairs_mut().extend_pairs([
            ("startDateTime", start.as_str()),
            ("endDateTime", end.as_str()),
            ("$top", "250"),
            ("$orderby", "start/dateTime"),
        ]);
    }
    let base = url.to_string();
    let mut next = base.clone();
    let mut rows = vec![];
    for _ in 0..100 {
        let v = state.api(a, "GET", &next, Value::Null).await?;
        rows.extend(
            array(if google(a) { &v["items"] } else { &v["value"] })
                .iter()
                .filter(|x| x["status"] != "cancelled" && x["isCancelled"] != true)
                .map(|x| normalize_event(x, google(a), &calendar)),
        );
        if rows.len() > MAX_ROWS {
            return Err("Calendar period exceeds 20,000 events. Choose a shorter period.".into());
        }
        let token = field(
            &v,
            if google(a) {
                "nextPageToken"
            } else {
                "@odata.nextLink"
            },
        );
        if token.is_empty() {
            break;
        }
        next = if google(a) {
            format!("{base}&pageToken={}", segment(&token))
        } else {
            graph_next(&token)?
        };
    }
    Ok(json!(rows))
}
fn event_attendees(event: &Value) -> Result<Vec<String>> {
    if let Some(xs) = event["attendees"].as_array() {
        let mut out = vec![];
        for x in xs {
            let value = x
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| field(x, "email"));
            let mut addresses = crate::addresses(&value)?;
            if addresses.len() != 1 {
                return Err("Each attendee must have one email address".into());
            }
            out.push(addresses.remove(0));
        }
        if out.len() > 200 {
            return Err("A meeting supports up to 200 attendees".into());
        }
        Ok(out)
    } else {
        crate::addresses(&field(event, "attendees"))
    }
}
fn microsoft_recurrence(value: &Value, start: &str) -> Result<Value> {
    if value.is_object() {
        return Ok(value.clone());
    }
    let rules = array(value);
    if rules.is_empty() {
        return Ok(Value::Null);
    }
    if rules.len() != 1 {
        return Err(
            "Microsoft recurrence supports one RRULE; edit complex recurrence at the provider"
                .into(),
        );
    }
    let rule = rules[0]
        .as_str()
        .unwrap_or("")
        .strip_prefix("RRULE:")
        .ok_or("Use an RRULE recurrence")?;
    let mut parts = std::collections::HashMap::new();
    for part in rule.split(';') {
        let (k, v) = part.split_once('=').ok_or("Invalid recurrence rule")?;
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
            return Err(format!(
                "Microsoft recurrence does not support {k} in this editor"
            ));
        }
        parts.insert(k, v);
    }
    let interval = parts
        .get("INTERVAL")
        .unwrap_or(&"1")
        .parse::<u64>()
        .map_err(|_| "Invalid recurrence interval")?;
    if interval == 0 || interval > 999 {
        return Err("Recurrence interval must be 1–999".into());
    }
    let start_date =
        NaiveDate::parse_from_str(&start.chars().take(10).collect::<String>(), "%Y-%m-%d")
            .map_err(err)?;
    use chrono::Datelike;
    let weekdays = [
        ("MO", "monday"),
        ("TU", "tuesday"),
        ("WE", "wednesday"),
        ("TH", "thursday"),
        ("FR", "friday"),
        ("SA", "saturday"),
        ("SU", "sunday"),
    ];
    let days = parts
        .get("BYDAY")
        .map(|s| {
            s.split(',')
                .map(|day| {
                    weekdays
                        .iter()
                        .find(|(d, _)| *d == day)
                        .map(|(_, name)| json!(name))
                        .ok_or("Use simple weekday names in Microsoft recurrence")
                })
                .collect::<std::result::Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![json!(start_date.format("%A").to_string().to_lowercase())]);
    let pattern = match parts.get("FREQ").copied().unwrap_or("") {
        "DAILY" => json!({"type":"daily","interval":interval}),
        "WEEKLY" => {
            json!({"type":"weekly","interval":interval,"daysOfWeek":days,"firstDayOfWeek":"monday"})
        }
        "MONTHLY" => {
            json!({"type":"absoluteMonthly","interval":interval,"dayOfMonth":parts.get("BYMONTHDAY").and_then(|x|x.parse::<u32>().ok()).unwrap_or(start_date.day())})
        }
        "YEARLY" => {
            json!({"type":"absoluteYearly","interval":interval,"dayOfMonth":parts.get("BYMONTHDAY").and_then(|x|x.parse::<u32>().ok()).unwrap_or(start_date.day()),"month":parts.get("BYMONTH").and_then(|x|x.parse::<u32>().ok()).unwrap_or(start_date.month())})
        }
        _ => return Err("Use daily, weekly, monthly, or yearly recurrence".into()),
    };
    let range = if let Some(count) = parts.get("COUNT") {
        let count = count
            .parse::<u64>()
            .map_err(|_| "Invalid recurrence count")?;
        if count == 0 || count > 999 {
            return Err("Recurrence count must be 1–999".into());
        }
        json!({"type":"numbered","startDate":start_date.to_string(),"numberOfOccurrences":count})
    } else if let Some(until) = parts.get("UNTIL") {
        let until = until.chars().take(8).collect::<String>();
        let date = NaiveDate::parse_from_str(&until, "%Y%m%d")
            .map_err(|_| "Invalid recurrence end date")?;
        json!({"type":"endDate","startDate":start_date.to_string(),"endDate":date.to_string()})
    } else {
        json!({"type":"noEnd","startDate":start_date.to_string()})
    };
    Ok(json!({"pattern":pattern,"range":range}))
}
fn event_body(event: &Value, g: bool) -> Result<Value> {
    let title = bounded_text(event, "title", 1000)?;
    if title.trim().is_empty() {
        return Err("Enter an event title".into());
    }
    let description = bounded_text(event, "description", 100_000)?;
    let location = bounded_text(event, "location", 4000)?;
    let start = field(event, "start");
    let end = field(event, "end");
    let all_day = event["allDay"].as_bool().unwrap_or(false);
    let (start_utc, end_utc) = if all_day {
        let s = NaiveDate::parse_from_str(&start.chars().take(10).collect::<String>(), "%Y-%m-%d")
            .map_err(|_| "Invalid all-day start")?;
        let e = NaiveDate::parse_from_str(&end.chars().take(10).collect::<String>(), "%Y-%m-%d")
            .map_err(|_| "Invalid all-day end")?;
        if e <= s {
            return Err("All-day end must be after start (exclusive)".into());
        }
        (
            s.and_hms_opt(0, 0, 0).unwrap().and_utc(),
            e.and_hms_opt(0, 0, 0).unwrap().and_utc(),
        )
    } else {
        let s = DateTime::parse_from_rfc3339(&start)
            .map_err(|_| "Start time must include a time-zone offset")?
            .with_timezone(&Utc);
        let e = DateTime::parse_from_rfc3339(&end)
            .map_err(|_| "End time must include a time-zone offset")?
            .with_timezone(&Utc);
        if e <= s {
            return Err("End time must be after start".into());
        }
        (s, e)
    };
    let attendees = event_attendees(event)?;
    let reminders = array(&event["reminders"])
        .iter()
        .map(|x| {
            x.as_u64()
                .filter(|m| *m <= 40320)
                .ok_or("Reminders must be 0–40320 minutes before the event")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if reminders.len() > 5 {
        return Err("Use at most five event reminders".into());
    }
    if g {
        let mut body = json!({"summary":title,"description":description,"location":location,"start":if all_day{json!({"date":start_utc.format("%Y-%m-%d").to_string()})}else{json!({"dateTime":start_utc.to_rfc3339(),"timeZone":event["timezone"].as_str().unwrap_or("UTC")})},"end":if all_day{json!({"date":end_utc.format("%Y-%m-%d").to_string()})}else{json!({"dateTime":end_utc.to_rfc3339(),"timeZone":event["timezone"].as_str().unwrap_or("UTC")})},"attendees":attendees.iter().map(|e|json!({"email":e})).collect::<Vec<_>>()});
        if event.get("recurrence").is_some() {
            if !event["recurrence"].is_null() && !event["recurrence"].is_array() {
                return Err("Google recurrence must be an RRULE list".into());
            }
            body["recurrence"] = event["recurrence"].clone();
        }
        if event.get("reminders").is_some() {
            body["reminders"] = json!({"useDefault":false,"overrides":reminders.iter().map(|m|json!({"method":"popup","minutes":m})).collect::<Vec<_>>()});
        }
        if event["onlineMeeting"] == true && field(event, "id").is_empty() {
            body["conferenceData"] = json!({"createRequest":{"requestId":format!("yas-{}",rand::random::<u64>()),"conferenceSolutionKey":{"type":"hangoutsMeet"}}});
        }
        Ok(body)
    } else {
        let mut body = json!({"subject":title,"body":{"contentType":"Text","content":description},"location":{"displayName":location},"start":{"dateTime":start_utc.format("%Y-%m-%dT%H:%M:%S").to_string(),"timeZone":"UTC"},"end":{"dateTime":end_utc.format("%Y-%m-%dT%H:%M:%S").to_string(),"timeZone":"UTC"},"isAllDay":all_day,"attendees":attendees.iter().map(|e|json!({"emailAddress":{"address":e},"type":"required"})).collect::<Vec<_>>()});
        if event.get("recurrence").is_some() {
            body["recurrence"] = microsoft_recurrence(&event["recurrence"], &start)?;
        }
        if event.get("reminders").is_some() {
            body["isReminderOn"] = json!(!reminders.is_empty());
            body["reminderMinutesBeforeStart"] = json!(reminders.first().copied().unwrap_or(15));
        }
        if event["onlineMeeting"] == true && field(event, "id").is_empty() {
            body["isOnlineMeeting"] = json!(true);
            body["onlineMeetingProvider"] = json!("teamsForBusiness");
        }
        Ok(body)
    }
}
pub async fn save_event(state: &App, a: &Account, event: Value, delete: bool) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::save_event(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            event,
            delete,
        )
        .await;
    }
    crate::require_groupware(a, "Calendar")?;
    let id = field(&event, "id");
    let calendar = calendar_id(event["calendarId"].as_str());
    let base = event_base(a, &calendar);
    let url = if id.is_empty() {
        base
    } else {
        format!("{base}/{}", segment(&id))
    };
    if delete {
        if id.is_empty() {
            return Err("Choose a saved event to delete".into());
        }
        return state
            .api(
                a,
                "DELETE",
                &if google(a) {
                    format!("{url}?sendUpdates=all")
                } else {
                    url
                },
                Value::Null,
            )
            .await;
    }
    let body = event_body(&event, google(a))?;
    let value = state
        .api(
            a,
            if id.is_empty() { "POST" } else { "PATCH" },
            &if google(a) {
                format!("{url}?sendUpdates=all&conferenceDataVersion=1")
            } else {
                url
            },
            body,
        )
        .await?;
    Ok(normalize_event(&value, google(a), &calendar))
}
pub async fn respond_event(
    state: &App,
    a: &Account,
    calendar: Option<String>,
    id: String,
    response: String,
) -> Result<Value> {
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::respond_event(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            id,
            response,
        )
        .await;
    }
    crate::require_groupware(a, "Calendar")?;
    if !["accepted", "tentative", "declined"].contains(&response.as_str()) || id.is_empty() {
        return Err("Choose Accept, Tentative, or Decline for a saved event".into());
    }
    let base = event_base(a, &calendar_id(calendar.as_deref()));
    let url = format!("{base}/{}", segment(&id));
    if google(a) {
        let event = state.api(a, "GET", &url, Value::Null).await?;
        let mut attendees = array(&event["attendees"]).to_vec();
        let mut found = false;
        for x in &mut attendees {
            if x["self"] == true
                || x["email"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(&a.email))
            {
                x["responseStatus"] = json!(response);
                found = true;
            }
        }
        if !found {
            return Err("Your account is not an attendee of this invitation".into());
        }
        state
            .api(
                a,
                "PATCH",
                &format!("{url}?sendUpdates=all"),
                json!({"attendees":attendees}),
            )
            .await?;
    } else {
        let action = match response.as_str() {
            "accepted" => "accept",
            "tentative" => "tentativelyAccept",
            _ => "decline",
        };
        state
            .api(
                a,
                "POST",
                &format!("{url}/{action}"),
                json!({"sendResponse":true}),
            )
            .await?;
    }
    Ok(json!({"response":response}))
}
#[tauri::command]
pub async fn calendar_sources(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
) -> Result<Value> {
    let a = owned(&window, &state, &account).await?;
    calendars(&state, &a).await
}
#[tauri::command]
pub async fn calendar_events(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    calendar: Option<String>,
    start: String,
    end: String,
) -> Result<Value> {
    let a = owned(&window, &state, &account).await?;
    events(&state, &a, start, end, calendar).await
}
#[tauri::command]
pub async fn event_respond(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    calendar: Option<String>,
    id: String,
    response: String,
) -> Result<Value> {
    let a = owned(&window, &state, &account).await?;
    respond_event(&state, &a, calendar, id, response).await
}

fn ics_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn ics_unescape(value: &str) -> String {
    let mut out = String::new();
    let mut escape = false;
    for ch in value.chars() {
        if escape {
            out.push(if ch == 'n' || ch == 'N' { '\n' } else { ch });
            escape = false;
        } else if ch == '\\' {
            escape = true;
        } else {
            out.push(ch);
        }
    }
    if escape {
        out.push('\\');
    }
    out
}
fn fold_ics(value: &str) -> String {
    let mut out = String::new();
    let mut bytes = 0;
    for ch in value.chars() {
        if bytes + ch.len_utf8() > 73 {
            out.push_str("\r\n ");
            bytes = 1;
        }
        out.push(ch);
        bytes += ch.len_utf8();
    }
    out.push_str("\r\n");
    out
}
fn parse_ics_time(value: &str, all_day: bool, tz: Option<&str>) -> Result<String> {
    if all_day {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .map(|d| d.to_string())
            .map_err(|_| "Invalid all-day ICS date".into());
    }
    if value.ends_with('Z') {
        return chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
            .map(|d| d.and_utc().to_rfc3339())
            .map_err(|_| "Invalid ICS date-time".into());
    }
    let zone=tz.ok_or("This ICS uses floating times without a time zone. Export it with UTC or a named time zone; Yas Mail will not guess event times.")?.trim_matches('"');
    let zone = match zone {
        "Arabian Standard Time" => "Asia/Dubai",
        "Arab Standard Time" => "Asia/Riyadh",
        "GMT Standard Time" => "Europe/London",
        "Pacific Standard Time" => "America/Los_Angeles",
        "Eastern Standard Time" => "America/New_York",
        "Central Standard Time" => "America/Chicago",
        "India Standard Time" => "Asia/Kolkata",
        "Tokyo Standard Time" => "Asia/Tokyo",
        "AUS Eastern Standard Time" => "Australia/Sydney",
        other => other,
    };
    let zone = zone.parse::<chrono_tz::Tz>().map_err(|_| {
        "This ICS uses an unsupported custom time zone. Export it with UTC or an IANA time zone."
    })?;
    let local = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .map_err(|_| "Invalid ICS date-time")?;
    use chrono::TimeZone;
    zone.from_local_datetime(&local).single().map(|d|d.with_timezone(&Utc).to_rfc3339()).ok_or("This ICS time is ambiguous or does not exist during a daylight-saving change. Export it with an explicit UTC time.".into())
}
pub fn parse_ics(content: &str) -> Result<Vec<Value>> {
    if content.len() > 5_000_000 {
        return Err("Choose an ICS file up to 5 MB".into());
    }
    let mut lines: Vec<String> = vec![];
    for line in content.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(previous) = lines.last_mut() {
                previous.push_str(&line[1..]);
            }
        } else {
            lines.push(line.trim_end_matches('\r').into());
        }
    }
    let mut events = vec![];
    let mut current: Option<Value> = None;
    let mut depth = 0u32;
    for line in lines {
        if line == "BEGIN:VEVENT" {
            if current.is_some() {
                return Err("Nested ICS events are invalid".into());
            }
            current = Some(
                json!({"title":"","description":"","location":"","attendees":[],"reminders":[],"recurrence":[]}),
            );
            depth = 0;
            continue;
        }
        if line == "END:VEVENT" {
            let e = current.take().ok_or("Unexpected ICS event end")?;
            event_body(&e, true)?;
            events.push(e);
            if events.len() > 500 {
                return Err("Import up to 500 events at once".into());
            }
            continue;
        }
        let Some(event) = current.as_mut() else {
            continue;
        };
        if line.starts_with("BEGIN:") {
            depth += 1;
            if depth > 10 {
                return Err("ICS nesting exceeds limit".into());
            }
            continue;
        }
        if line.starts_with("END:") {
            depth = depth.saturating_sub(1);
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let property = name.split(';').next().unwrap_or("");
        if depth > 0 {
            if property == "TRIGGER" {
                if let Some(minutes) = value
                    .strip_prefix("-PT")
                    .and_then(|s| s.strip_suffix('M'))
                    .and_then(|s| s.parse::<u64>().ok())
                {
                    event["reminders"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!(minutes));
                }
            }
            continue;
        }
        match property {
            "SUMMARY" => event["title"] = json!(ics_unescape(value)),
            "DESCRIPTION" => event["description"] = json!(ics_unescape(value)),
            "LOCATION" => event["location"] = json!(ics_unescape(value)),
            "DTSTART" | "DTEND" => {
                let all_day = name.split(';').any(|p| p == "VALUE=DATE");
                let tz = name.split(';').find_map(|p| p.strip_prefix("TZID="));
                event[if property == "DTSTART" {
                    "start"
                } else {
                    "end"
                }] = json!(parse_ics_time(value, all_day, tz)?);
                if all_day {
                    event["allDay"] = json!(true);
                }
            }
            "RRULE" => event["recurrence"]
                .as_array_mut()
                .unwrap()
                .push(json!(format!("RRULE:{value}"))),
            "ATTENDEE" => {
                if let Some(email) = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                {
                    crate::addresses(email)?;
                    event["attendees"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"email":email}));
                }
            }
            "UID" => event["importUid"] = json!(value),
            _ => {}
        }
    }
    if current.is_some() {
        return Err("The ICS event is incomplete".into());
    }
    if events.is_empty() {
        return Err("No supported events found in this ICS file".into());
    }
    Ok(events)
}
pub fn export_ics(events: &[Value]) -> Result<String> {
    if events.len() > 20_000 {
        return Err("Too many events to export".into());
    }
    let mut out="BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Yas Mail//Calendar//EN\r\nCALSCALE:GREGORIAN\r\n".to_string();
    for event in events {
        event_body(event, true)?;
        out.push_str("BEGIN:VEVENT\r\n");
        out.push_str(&fold_ics(&format!(
            "UID:{}@yas-mail.local",
            ics_escape(event["id"].as_str().unwrap_or("local"))
        )));
        out.push_str(&fold_ics(&format!(
            "DTSTAMP:{}",
            Utc::now().format("%Y%m%dT%H%M%SZ")
        )));
        for (key, property) in [
            ("title", "SUMMARY"),
            ("description", "DESCRIPTION"),
            ("location", "LOCATION"),
        ] {
            out.push_str(&fold_ics(&format!(
                "{property}:{}",
                ics_escape(&field(event, key))
            )));
        }
        for (key, property) in [("start", "DTSTART"), ("end", "DTEND")] {
            let value = field(event, key);
            let line = if event["allDay"] == true {
                format!(
                    "{property};VALUE=DATE:{}",
                    value
                        .chars()
                        .take(10)
                        .filter(|c| *c != '-')
                        .collect::<String>()
                )
            } else {
                let time = DateTime::parse_from_rfc3339(&value)
                    .map_err(|_| "Export requires event time-zone offsets")?;
                format!(
                    "{property}:{}",
                    time.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ")
                )
            };
            out.push_str(&fold_ics(&line));
        }
        for email in event_attendees(event)? {
            out.push_str(&fold_ics(&format!("ATTENDEE:mailto:{email}")));
        }
        for rule in array(&event["recurrence"]).iter().filter_map(Value::as_str) {
            if rule.starts_with("RRULE:") && !rule.contains(['\r', '\n']) {
                out.push_str(&fold_ics(rule));
            }
        }
        for minutes in array(&event["reminders"]).iter().filter_map(Value::as_u64) {
            out.push_str(&format!("BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Event reminder\r\nTRIGGER:-PT{minutes}M\r\nEND:VALARM\r\n"));
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    if out.len() > 10_000_000 {
        return Err("Calendar export exceeds 10 MB".into());
    }
    Ok(out)
}
#[tauri::command]
pub fn calendar_parse_ics(content: String) -> Result<Value> {
    Ok(json!(parse_ics(&content)?))
}
#[tauri::command]
pub fn calendar_export_ics(events: Vec<Value>) -> Result<String> {
    export_ics(&events)
}

fn photo_type(bytes: &[u8]) -> Result<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok("image/jpeg")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok("image/webp")
    } else {
        Err("Use a PNG, JPEG, or WebP contact photo".into())
    }
}
async fn bounded_photo(mut response: reqwest::Response) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err("The provider could not return this contact photo".into());
    }
    if response.content_length().is_some_and(|n| n > 3_000_000) {
        return Err("Contact photo exceeds 3 MB".into());
    }
    let mut bytes = vec![];
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Contact photo download was interrupted")?
    {
        if bytes.len() + chunk.len() > 3_000_000 {
            return Err("Contact photo exceeds 3 MB".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    photo_type(&bytes)?;
    Ok(bytes)
}
#[tauri::command]
pub async fn contact_photo(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    id: String,
) -> Result<Value> {
    use base64::Engine;
    let a = owned(&window, &state, &account).await?;
    if let Some(settings) = a.exchange.clone() {
        let (bytes, _) = crate::exchange::contact_photo(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            id,
        )
        .await?;
        if bytes.len() > 3_000_000 {
            return Err("Contact photo exceeds 3 MB".into());
        }
        return Ok(
            json!({"data":base64::engine::general_purpose::STANDARD.encode(&bytes),"contentType":photo_type(&bytes)?}),
        );
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(err)?;
    let response = if google(&a) {
        let value = state
            .api(
                &a,
                "GET",
                &format!("{PEOPLE}/{}?personFields=photos", person_resource(&id)?),
                Value::Null,
            )
            .await?;
        let url = array(&value["photos"])
            .iter()
            .find(|x| x["default"] != true)
            .and_then(|x| x["url"].as_str())
            .ok_or("This contact has no photo")?;
        let url = url::Url::parse(url).map_err(|_| "Invalid Google contact photo URL")?;
        let host = url.host_str().unwrap_or("");
        if url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || !(host == "googleusercontent.com" || host.ends_with(".googleusercontent.com"))
        {
            return Err("Google returned an unsupported contact photo host".into());
        }
        client
            .get(url)
            .send()
            .await
            .map_err(|_| "Contact photo could not be downloaded")?
    } else {
        client
            .get(format!("{GRAPH}/contacts/{}/photo/$value", segment(&id)))
            .bearer_auth(state.token(&a).await?)
            .send()
            .await
            .map_err(|_| "Contact photo could not be downloaded")?
    };
    let bytes = bounded_photo(response).await?;
    crate::windows::check_owner(&state, window.label(), &account)?;
    Ok(
        json!({"data":base64::engine::general_purpose::STANDARD.encode(&bytes),"contentType":photo_type(&bytes)?}),
    )
}
#[tauri::command]
pub async fn contact_photo_save(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    id: String,
    data: String,
    content_type: String,
) -> Result<Value> {
    use base64::Engine;
    let a = owned(&window, &state, &account).await?;
    if data.len() > 4_000_000 {
        return Err("Choose a contact photo up to 3 MB".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| "Invalid photo encoding")?;
    if bytes.len() > 3_000_000 || photo_type(&bytes)? != content_type {
        return Err("Photo content type does not match its bytes".into());
    }
    if let Some(settings) = a.exchange.clone() {
        return crate::exchange::contact_photo_save(
            settings,
            crate::mail_credentials::load(&format!("{}:exchange", a.id))?,
            id,
            bytes,
            content_type,
        )
        .await;
    }
    if google(&a) {
        state
            .api(
                &a,
                "PATCH",
                &format!("{PEOPLE}/{}:updateContactPhoto", person_resource(&id)?),
                json!({"photoBytes":base64::engine::general_purpose::STANDARD.encode(bytes)}),
            )
            .await?;
    } else {
        let response = state
            .http
            .put(format!("{GRAPH}/contacts/{}/photo/$value", segment(&id)))
            .bearer_auth(state.token(&a).await?)
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await
            .map_err(|_| "Contact photo upload failed")?;
        if !response.status().is_success() {
            return Err("Microsoft did not accept the contact photo".into());
        }
    }
    Ok(json!({"saved":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn microsoft_continuations_stay_in_owned_graph_api() {
        assert!(graph_next("https://graph.microsoft.com/v1.0/me/contacts?$skip=100").is_ok());
        for x in [
            "https://evil.invalid/v1.0/me/contacts",
            "http://graph.microsoft.com/v1.0/me/contacts",
            "https://graph.microsoft.com/v1.0/users/other/contacts",
            "https://graph.microsoft.com:444/v1.0/me/contacts",
        ] {
            assert!(graph_next(x).is_err());
        }
    }
    #[test]
    fn richer_contacts_keep_multiple_addresses_and_business_fields() {
        let google = google_contact(
            &json!({"resourceName":"people/1","names":[{"displayName":"Alex"}],"emailAddresses":[{"value":"one@example.com"},{"value":"two@example.com"}],"phoneNumbers":[{"value":"111"},{"value":"222"}],"organizations":[{"name":"Example","title":"Engineer"}],"biographies":[{"value":"Notes"}]}),
        );
        assert_eq!(google["emails"].as_array().unwrap().len(), 2);
        assert_eq!(google["company"], "Example");
        let ms = microsoft_contact(
            &json!({"id":"m","displayName":"Alex","emailAddresses":[{"address":"a@example.com"}],"mobilePhone":"123","businessPhones":["456"],"companyName":"Example","businessAddress":{"city":"Dubai"}}),
            Some("folder"),
        );
        assert_eq!(ms["phones"].as_array().unwrap().len(), 2);
        assert_eq!(ms["addresses"][0]["city"], "Dubai");
        assert_eq!(ms["groups"][0], "folder");
    }
    #[test]
    fn meeting_body_has_real_provider_conference_and_reminder_fields() {
        let e = json!({"title":"Review","start":"2026-11-10T09:00:00Z","end":"2026-11-10T10:00:00Z","attendees":"a@example.com","onlineMeeting":true,"recurrence":["RRULE:FREQ=WEEKLY;COUNT=5;BYDAY=TU"],"reminders":[15]});
        let g = event_body(&e, true).unwrap();
        assert_eq!(
            g["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"],
            "hangoutsMeet"
        );
        let m = event_body(&e, false).unwrap();
        assert_eq!(m["onlineMeetingProvider"], "teamsForBusiness");
        assert_eq!(m["recurrence"]["pattern"]["type"], "weekly");
        assert_eq!(m["recurrence"]["range"]["numberOfOccurrences"], 5);
        assert!(event_body(
            &json!({"title":"bad","start":"2026-11-10T10:00:00Z","end":"2026-11-10T09:00:00Z"}),
            true
        )
        .is_err());
    }
    #[test]
    fn ics_roundtrip_preserves_unicode_recurrence_attendees_and_alarms() {
        let e = json!({"id":"123","title":"Café, review; next","start":"2026-11-10T09:00:00Z","end":"2026-11-10T10:00:00Z","description":"Line one\nLine two","attendees":[{"email":"a@example.com"}],"reminders":[10],"recurrence":["RRULE:FREQ=DAILY;COUNT=3"]});
        let encoded = export_ics(&[e]).unwrap();
        let decoded = parse_ics(&encoded).unwrap();
        assert_eq!(decoded[0]["title"], "Café, review; next");
        assert_eq!(decoded[0]["description"], "Line one\nLine two");
        assert_eq!(decoded[0]["reminders"][0], 10);
        assert_eq!(decoded[0]["attendees"][0]["email"], "a@example.com");
        assert_eq!(decoded[0]["recurrence"][0], "RRULE:FREQ=DAILY;COUNT=3");
    }
    #[test]
    fn ics_resolves_named_time_zones_and_refuses_to_guess_dst_ambiguity() {
        assert_eq!(
            parse_ics_time("20260710T090000", false, Some("Europe/London")).unwrap(),
            "2026-07-10T08:00:00+00:00"
        );
        assert_eq!(
            parse_ics_time("20261110T090000", false, Some("Arabian Standard Time")).unwrap(),
            "2026-11-10T05:00:00+00:00"
        );
        assert!(parse_ics_time("20261110T090000", false, None)
            .unwrap_err()
            .contains("time zone"));
        assert!(
            parse_ics_time("20261101T013000", false, Some("America/New_York"))
                .unwrap_err()
                .contains("ambiguous")
        );
    }
    #[test]
    fn microsoft_complex_recurrence_is_not_silently_rewritten() {
        assert!(microsoft_recurrence(
            &json!(["RRULE:FREQ=MONTHLY;BYDAY=2MO"]),
            "2026-11-10T09:00:00Z"
        )
        .is_err());
        assert!(microsoft_recurrence(
            &json!(["RRULE:FREQ=WEEKLY;INTERVAL=0"]),
            "2026-11-10T09:00:00Z"
        )
        .is_err());
    }
}
