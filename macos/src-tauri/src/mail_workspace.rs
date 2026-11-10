//! Window-scoped offline mail workspace and durable send/reminder workers.
use crate::{mail_store::MailStore, Account, App, Compose, Result};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json,Value};
use std::sync::Arc;
use tauri::Emitter;
use tauri_plugin_notification::NotificationExt;

fn now()->i64{Utc::now().timestamp()}
fn date(value:&str)->Result<i64>{chrono::DateTime::parse_from_rfc3339(value).map(|v|v.timestamp()).map_err(|_|"Choose a valid date and time.".into())}
fn offset(cursor:Option<&str>)->Result<usize>{let n=cursor.unwrap_or("0").parse::<usize>().map_err(|_|"Invalid cached mail page.")?;if n>1_000_000{return Err("Mail page offset is too large.".into())}Ok(n)}
async fn owned(state:&App,label:&str,account:Option<&str>)->Result<Vec<Account>>{
    let ids={let registry=state.windows.lock().map_err(crate::err)?;if !registry.seeds.contains_key(label){return Err("Window is no longer registered".into())}registry.owners.iter().filter(|(_,owner)|owner.as_str()==label).map(|(id,_)|id.clone()).collect::<Vec<_>>()};
    if let Some(account)=account{crate::windows::check_owner(state,label,account)?;return Ok(vec![state.account(account).await?]);}
    Ok(state.accounts.lock().await.iter().filter(|a|ids.contains(&a.id)).cloned().collect())
}
fn account_ids(accounts:&[Account])->Vec<String>{accounts.iter().map(|a|a.id.clone()).collect()}
fn enrich(page:&mut Value,accounts:&[Account]){
    if let Some(items)=page["items"].as_array_mut(){for item in items{if let Some(account)=accounts.iter().find(|a|item["accountId"]==a.id){item["accountEmail"]=json!(account.email);item["provider"]=json!(account.provider);}}}
}
fn unread_count(items:&[Value])->usize{items.iter().filter(|m|m["unread"].as_bool().unwrap_or(false)).count()}

/// A cached page opens immediately. Explicit refresh and cache misses fetch live data.
pub async fn list(state:&App,store:&MailStore,accounts:Vec<Account>,folder:String,query:String,cursor:Option<String>,refresh:bool)->Result<Value>{
    let start=offset(cursor.as_deref())?;
    let ids=account_ids(&accounts);
    if ["snoozed","reminders","local-archive"].contains(&folder.as_str()){
        let mut result=store.list(&ids,&folder,&query,start)?;
        let reminders=if folder=="local-archive"{vec![]}else{store.reminder_rows(&ids,Some(if folder=="snoozed"{"snooze"}else{"followup"}))?};
        if let Some(items)=result["items"].as_array_mut(){for item in items{if let Some(reminder)=reminders.iter().find(|r|r["accountId"]==item["accountId"]&&r["id"]==item["id"]){item["reminder"]=reminder.clone();}}}
        enrich(&mut result,&accounts);return Ok(result)
    }
    let mut errors=vec![];
    let mut fetched=false;
    for account in &accounts{
        if account.provider=="smtp"{continue;}
        let query_folder=if query.trim().is_empty(){folder.clone()}else if folder=="search"{"search".into()}else{folder.clone()};
        let missing=!store.has_folder(&account.id,&folder)?;
        if refresh||missing{
            match crate::fetch_mail(state,account,query_folder,query.clone(),None).await{
                Ok(page)=>{
                    let items=page["items"].as_array().cloned().unwrap_or_default();
                    let next=page["cursor"].as_str();
                    if let Err(error)=store.cache_page(&account.id,&folder,&items,next.is_none()&&query.trim().is_empty()){errors.push(json!({"accountId":account.id,"error":error}));}
                    if query.trim().is_empty(){store.set_backfill(&account.id,&folder,next)?;}
                    fetched=true;
                },
                Err(error)=>errors.push(json!({"accountId":account.id,"error":error}))
            }
        }
    }
    let mut result=store.list(&ids,&folder,&query,start)?;
    // A Load more request may advance the provider's saved backfill cursor.
    if start>0&&query.trim().is_empty()&&result["items"].as_array().is_none_or(|v|v.len()<50){
        for account in &accounts{
            let (cursor,complete)=store.backfill(&account.id,&folder)?;
            if complete||cursor.is_none(){continue;}
            match crate::fetch_mail(state,account,folder.clone(),String::new(),cursor).await{
                Ok(page)=>{let items=page["items"].as_array().cloned().unwrap_or_default();store.cache_page(&account.id,&folder,&items,false)?;store.set_backfill(&account.id,&folder,page["cursor"].as_str())?;fetched=true;},
                Err(error)=>errors.push(json!({"accountId":account.id,"error":error}))
            }
        }
        result=store.list(&ids,&folder,&query,start)?;
    }
    if result["cursor"].is_null()&&query.trim().is_empty(){
        let incomplete=accounts.iter().any(|a|store.backfill(&a.id,&folder).is_ok_and(|(cursor,complete)|!complete&&cursor.is_some()));
        let count=result["items"].as_array().map(Vec::len).unwrap_or(0);
        if incomplete&&count>0{result["cursor"]=json!((start+count).to_string());}
    }
    result["cached"]=json!(!fetched);result["offline"]=json!(!errors.is_empty()&&!fetched);result["errors"]=json!(errors);
    result["searchScope"]=json!("downloaded mail; refresh also searches the provider where supported");
    enrich(&mut result,&accounts);
    Ok(result)
}

#[tauri::command]
pub async fn workspace_mail_list(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,folder:String,query:String,cursor:Option<String>,refresh:Option<bool>)->Result<Value>{let accounts=owned(&state,window.label(),None).await?;let result=list(&state,&store,accounts.clone(),folder,query,cursor,refresh.unwrap_or(false)).await?;for account in accounts{crate::windows::check_owner(&state,window.label(),&account.id)?;}Ok(result)}
#[tauri::command]
pub async fn workspace_sync(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:Option<String>)->Result<Value>{let accounts=owned(&state,window.label(),account.as_deref()).await?;let mut results=vec![];for account in accounts{let result=sync_one(&state,&store,&account).await;results.push(match result{Ok((new,count))=>json!({"accountId":account.id,"ok":true,"newMessages":new.len(),"cachedMessages":count}),Err(error)=>json!({"accountId":account.id,"ok":false,"error":error})});}Ok(json!(results))}
#[tauri::command]
pub async fn workspace_settings(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,settings:Option<Value>)->Result<Value>{owned(&state,window.label(),None).await?;if let Some(value)=&settings{let seconds=value["syncSeconds"].as_u64().unwrap_or(60);if !(30..=3600).contains(&seconds){return Err("Choose a background sync interval between 30 seconds and 60 minutes.".into())}let value=json!({"syncSeconds":seconds,"backgroundSync":value["backgroundSync"].as_bool().unwrap_or(true),"notifications":value["notifications"].as_bool().unwrap_or(true)});return store.settings(Some(&value))}store.settings(None)}

fn public_workspace_preferences(value:&Value)->Value{json!({"syncEnabled":value["backgroundSync"].as_bool().unwrap_or(true),"pollSeconds":value["syncSeconds"].as_u64().unwrap_or(60),"notifications":value["notifications"].as_bool().unwrap_or(false),"notificationPreview":value["notificationPreview"].as_bool().unwrap_or(false)})}
#[tauri::command]
pub async fn workspace_preferences_get(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>)->Result<Value>{owned(&state,window.label(),None).await?;Ok(public_workspace_preferences(&store.settings(None)?))}
#[tauri::command]
pub async fn workspace_preferences_save(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,prefs:Value)->Result<Value>{owned(&state,window.label(),None).await?;let seconds=prefs["pollSeconds"].as_u64().unwrap_or(60);if !(30..=3600).contains(&seconds){return Err("Choose a sync interval from 30 seconds to 60 minutes.".into())}let value=json!({"backgroundSync":prefs["syncEnabled"].as_bool().unwrap_or(true),"syncSeconds":seconds,"notifications":prefs["notifications"].as_bool().unwrap_or(false),"notificationPreview":prefs["notificationPreview"].as_bool().unwrap_or(false)});Ok(public_workspace_preferences(&store.settings(Some(&value))?))}
#[tauri::command]
pub async fn secure_preferences_get(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String)->Result<Value>{crate::windows::check_owner(&state,window.label(),&account)?;Ok(store.preference(&account)?.unwrap_or_else(||json!({})))}
#[tauri::command]
pub async fn secure_preferences_save(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,prefs:Value)->Result<Value>{crate::windows::check_owner(&state,window.label(),&account)?;store.save_preference(&account,&prefs)}

#[tauri::command]
pub async fn local_draft_save(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,id:Option<String>,draft:Compose)->Result<Value>{crate::windows::check_owner(&state,window.label(),&account)?;store.save_draft(&account,id.as_deref(),&serde_json::to_value(draft).map_err(crate::err)?)}
#[tauri::command]
pub async fn local_draft_list(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:Option<String>)->Result<Value>{let accounts=owned(&state,window.label(),account.as_deref()).await?;Ok(json!(store.drafts(&account_ids(&accounts))?))}
#[tauri::command]
pub async fn local_draft_delete(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,id:String)->Result<()>{crate::windows::check_owner(&state,window.label(),&account)?;store.delete_draft(&account,&id)}

fn validate_queued_draft(draft:&Compose)->Result<()>{let to=crate::addresses(&draft.to)?;crate::addresses(&draft.cc)?;crate::addresses(&draft.bcc)?;if to.is_empty(){return Err("Add at least one recipient before scheduling this message.".into())}if draft.subject.contains(['\r','\n']){return Err("Subject must be a single line.".into())}if draft.files.len()>100{return Err("Too many attachments.".into())}Ok(())}
#[tauri::command]
pub async fn mail_queue(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,draft:Compose,send_at:Option<String>,undo_seconds:Option<u64>)->Result<Value>{crate::windows::check_owner(&state,window.label(),&account)?;state.account(&account).await?;validate_queued_draft(&draft)?;let undo=undo_seconds.unwrap_or(5);if undo>30{return Err("Undo Send can delay a message by up to 30 seconds.".into())}let at=send_at.as_deref().filter(|s|!s.is_empty()).map(date).transpose()?.unwrap_or(now()+undo as i64).max(now()+undo as i64);store.queue(&account,&serde_json::to_value(draft).map_err(crate::err)?,at)}
#[tauri::command]
pub async fn outbox_list(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:Option<String>)->Result<Value>{let accounts=owned(&state,window.label(),account.as_deref()).await?;Ok(json!(store.jobs(&account_ids(&accounts))?))}
#[tauri::command]
pub async fn outbox_cancel(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,id:String)->Result<bool>{crate::windows::check_owner(&state,window.label(),&account)?;store.cancel_job(&account,&id)}

#[tauri::command]
pub async fn mail_snooze(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,id:String,until:String)->Result<()>{crate::windows::check_owner(&state,window.label(),&account)?;store.set_reminder(&account,&id,"snooze",if until.is_empty(){None}else{Some(date(&until)?)},"")}
#[tauri::command]
pub async fn mail_remind(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:String,id:String,at:String,note:Option<String>)->Result<()>{crate::windows::check_owner(&state,window.label(),&account)?;store.set_reminder(&account,&id,"followup",if at.is_empty(){None}else{Some(date(&at)?)},note.as_deref().unwrap_or(""))}
#[tauri::command]
pub async fn attachment_search(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,account:Option<String>,query:String)->Result<Value>{if query.len()>2000{return Err("Attachment search is too long.".into())}let accounts=owned(&state,window.label(),account.as_deref()).await?;let mut rows=store.attachment_search(&account_ids(&accounts),&query)?;for row in &mut rows{if let Some(account)=accounts.iter().find(|a|row["accountId"]==a.id){row["accountEmail"]=json!(account.email);}}Ok(json!(rows))}

#[derive(Deserialize)]
pub struct MailSelection{pub account:String,pub id:String}
#[tauri::command]
pub async fn mail_bulk_action(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,store:tauri::State<'_,Arc<MailStore>>,items:Vec<MailSelection>,action:String)->Result<Value>{
    if items.len()>200||items.is_empty(){return Err("Select between one and 200 messages.".into())}
    if !["read","unread","star","unstar","archive","trash"].contains(&action.as_str())&&!action.starts_with("move:"){return Err("Choose a supported mail action.".into())}
    // Validate all ownership first so a forged item cannot partially mutate another window.
    for item in &items{crate::windows::check_owner(&state,window.label(),&item.account)?;}
    let mut completed=vec![];let mut failed=vec![];
    for item in items{let account=state.account(&item.account).await?;let result=if let Some(folder)=action.strip_prefix("move:"){crate::mail_provider_tools::move_message(&state,&account,&item.id,folder).await}else{crate::perform_mail_action(&state,&account,item.id.clone(),action.clone()).await};match result{Ok(_)=>{store.apply_action(&item.account,&item.id,&action)?;completed.push(json!({"account":item.account,"id":item.id}));},Err(error)=>failed.push(json!({"account":item.account,"id":item.id,"error":error}))}}
    Ok(json!({"completed":completed,"failed":failed}))
}

async fn sync_one(state:&App,store:&MailStore,account:&Account)->Result<(Vec<Value>,usize)>{
    if account.provider=="smtp"{return Ok((vec![],0));}
    let was_seeded=store.has_folder(&account.id,"inbox")?;
    let page=crate::fetch_mail(state,account,"inbox".into(),String::new(),None).await?;
    let items=page["items"].as_array().cloned().unwrap_or_default();
    let next=page["cursor"].as_str();
    let new=store.cache_page(&account.id,"inbox",&items,next.is_none())?;
    if !was_seeded{store.set_backfill(&account.id,"inbox",next)?;}
    // Backfill one older page per cycle without starving newer messages or other accounts.
    let (cursor,complete)=store.backfill(&account.id,"inbox")?;
    if !complete{if let Some(cursor)=cursor{if let Ok(older)=crate::fetch_mail(state,account,"inbox".into(),String::new(),Some(cursor)).await{let rows=older["items"].as_array().cloned().unwrap_or_default();store.cache_page(&account.id,"inbox",&rows,false)?;store.set_backfill(&account.id,"inbox",older["cursor"].as_str())?;}}}
    // Client-side rules are explicit for protocols without a server rule API.
    if account.provider=="imap"{for rule in store.local_rules(&account.id)?{if !rule["enabled"].as_bool().unwrap_or(true){continue;}for item in &new{if crate::mail_provider_tools::rule_matches(&rule,item)&&store.claim_rule(&account.id,rule["id"].as_str().unwrap_or(""),item["id"].as_str().unwrap_or(""))?{let _=crate::mail_provider_tools::apply_rule(state,account,&rule,item["id"].as_str().unwrap_or("")).await;}}}}
    Ok((new,unread_count(&items)))
}
fn owner(state:&App,account:&str)->Option<String>{state.windows.lock().ok()?.owners.get(account).cloned()}
fn notify(app:&tauri::AppHandle,title:&str,body:&str){let _=app.notification().builder().title(title).body(body).show();}

pub fn start_background(app:tauri::AppHandle,state:Arc<App>,store:Arc<MailStore>){
    let sync_app=app.clone();let sync_state=state.clone();let sync_store=store.clone();
    tauri::async_runtime::spawn(async move{
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        loop{let settings=sync_store.settings(None).unwrap_or_else(|_|json!({}));if settings["backgroundSync"].as_bool().unwrap_or(true){
            let accounts=sync_state.accounts.lock().await.clone();
            for batch in accounts.chunks(2){let mut workers=tokio::task::JoinSet::new();for account in batch{let account=account.clone();let state=sync_state.clone();let store=sync_store.clone();workers.spawn(async move{let result=sync_one(&state,&store,&account).await;(account,result)});}while let Some(Ok((account,result)))=workers.join_next().await{if let Some(label)=owner(&sync_state,&account.id){match result{Ok((new,count))=>{let _=sync_app.emit_to(label,"yasmail:mail-synced",json!({"accountId":account.id,"newMessages":new.len(),"inboxUnread":count}));if !new.is_empty()&&settings["notifications"].as_bool().unwrap_or(true){notify(&sync_app,"New mail in Yas Mail",&format!("{} new message{} for {}",new.len(),if new.len()==1{""}else{"s"},account.email));}},Err(error)=>{let _=sync_app.emit_to(label,"yasmail:sync-error",json!({"accountId":account.id,"error":error}));}}}}}
        }tokio::time::sleep(std::time::Duration::from_secs(settings["syncSeconds"].as_u64().unwrap_or(60).clamp(30,3600))).await;}
    });
    let out_app=app.clone();let out_state=state.clone();let out_store=store.clone();
    tauri::async_runtime::spawn(async move{loop{match out_store.claim_due(){Ok(Some(job))=>{let account_id=job["accountId"].as_str().unwrap_or("").to_string();let id=job["id"].as_str().unwrap_or("").to_string();let result=match( out_state.account(&account_id).await,serde_json::from_value::<Compose>(job["draft"].clone())){(Ok(account),Ok(draft))=>crate::send_mail(&out_state,&account,draft,false).await,(Err(error),_)=>Err(error),(_,Err(_))=>Err("The saved scheduled message could not be read.".into())};let error=result.as_ref().err().map(String::as_str);let _=out_store.finish_job(&account_id,&id,result.is_ok(),error);if let Some(label)=owner(&out_state,&account_id){let _=out_app.emit_to(label,"yasmail:outbox-changed",json!({"accountId":account_id,"id":id,"state":if result.is_ok(){"accepted"}else{"uncertain"},"error":error}));}},Ok(None)=>{},Err(_)=>{}}tokio::time::sleep(std::time::Duration::from_secs(1)).await;}});
    tauri::async_runtime::spawn(async move{loop{if let Ok(reminders)=store.due_reminders(){for reminder in reminders{if let Some(account)=reminder["accountId"].as_str(){if let Some(label)=owner(&state,account){let _=app.emit_to(label,"yasmail:reminder",reminder.clone());if store.settings(None).ok().and_then(|v|v["notifications"].as_bool()).unwrap_or(true){notify(&app,if reminder["kind"]=="snooze"{"Snoozed email is ready"}else{"Email follow-up reminder"},reminder["note"].as_str().filter(|s|!s.is_empty()).unwrap_or("Open Yas Mail to review this message."));}}}}}tokio::time::sleep(std::time::Duration::from_secs(5)).await;}});
}

#[cfg(test)]
mod tests{use super::*;#[test]fn scheduling_rejects_invalid_recipients_and_header_injection(){let mut d=Compose::default();d.to="bad\r\nBcc: hidden@example.com".into();assert!(validate_queued_draft(&d).is_err());d.to="recipient@example.com".into();d.subject="bad\nsubject".into();assert!(validate_queued_draft(&d).is_err());d.subject="Draft".into();assert!(validate_queued_draft(&d).is_ok());}#[test]fn page_and_date_inputs_are_bounded(){assert!(offset(Some("1000001")).is_err());assert!(offset(Some("arbitrary-token")).is_err());assert_eq!(offset(None).unwrap(),0);assert!(date("not a date").is_err());assert_eq!(date("2026-11-09T10:00:00Z").unwrap(),1794218400);}}
