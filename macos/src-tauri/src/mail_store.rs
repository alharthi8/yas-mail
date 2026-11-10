//! Durable, account-scoped local mail state. Credentials are never stored here.
use crate::Result;
use chrono::Utc;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{path::Path, sync::Mutex, collections::HashSet};
use crate::cache_crypto::CacheCrypto;

const PAGE_SIZE: usize = 50;
const MAX_MESSAGE: usize = 8_000_000;
const MAX_DRAFT: usize = 30_000_000;

pub struct MailStore { connection: Mutex<Connection>, crypto:CacheCrypto }
fn sql_error(_: rusqlite::Error) -> String { "The local mail database could not complete this operation. Check available disk space and restart Yas Mail.".into() }
fn valid_id(value: &str) -> Result<()> { if value.is_empty() || value.len()>8192 || value.chars().any(char::is_control) { Err("Invalid mail identifier".into()) } else { Ok(()) } }
fn now() -> i64 { Utc::now().timestamp() }
fn key() -> String { format!("{:016x}{:016x}",rand::random::<u64>(),rand::random::<u64>()) }
fn context(kind:&str,account:&str,id:&str)->String{format!("{kind}:{}:{account}:{id}",account.len())}
fn fts_query(query: &str) -> String {
    query.split_whitespace().take(20).map(|part|format!("\"{}\"",part.replace('"',"\"\""))).collect::<Vec<_>>().join(" AND ")
}
fn placeholders(count: usize) -> String { std::iter::repeat_n("?",count).collect::<Vec<_>>().join(",") }
fn search_terms(query:&str)->Vec<String>{let mut words=vec![];let mut word=String::new();let mut quoted=false;for c in query.chars(){if c=='"'{quoted=!quoted;}else if c.is_whitespace()&&!quoted{if !word.is_empty(){words.push(std::mem::take(&mut word));}}else{word.push(c);}}if !word.is_empty(){words.push(word);}words.truncate(30);words}
fn like(value:&str)->String{format!("%{}%",value.replace('\\',"\\\\").replace('%',"\\%").replace('_',"\\_"))}

impl MailStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> { let crypto=CacheCrypto::open(path.as_ref())?;Self::from_connection(Connection::open(path).map_err(sql_error)?,crypto) }
    fn from_connection(connection: Connection,crypto:CacheCrypto) -> Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(3)).map_err(sql_error)?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON;
          CREATE TABLE IF NOT EXISTS messages(account TEXT NOT NULL,id TEXT NOT NULL,subject TEXT NOT NULL,sender TEXT NOT NULL,recipients TEXT NOT NULL,body TEXT NOT NULL,unread INTEGER NOT NULL DEFAULT 0,starred INTEGER NOT NULL DEFAULT 0,sort_at INTEGER NOT NULL,json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,id));
          CREATE INDEX IF NOT EXISTS message_dates ON messages(account,sort_at DESC);
          CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5(account UNINDEXED,id UNINDEXED,subject,sender,recipients,body,tokenize='unicode61');
          CREATE TABLE IF NOT EXISTS memberships(account TEXT NOT NULL,id TEXT NOT NULL,folder TEXT NOT NULL,PRIMARY KEY(account,id,folder),FOREIGN KEY(account,id) REFERENCES messages(account,id) ON DELETE CASCADE);
          CREATE INDEX IF NOT EXISTS folder_memberships ON memberships(account,folder);
          CREATE TABLE IF NOT EXISTS attachments(account TEXT NOT NULL,message_id TEXT NOT NULL,id TEXT NOT NULL,name TEXT NOT NULL,size INTEGER,json TEXT NOT NULL,PRIMARY KEY(account,message_id,id),FOREIGN KEY(account,message_id) REFERENCES messages(account,id) ON DELETE CASCADE);
          CREATE TABLE IF NOT EXISTS sync_state(account TEXT NOT NULL,folder TEXT NOT NULL,cursor TEXT,complete INTEGER NOT NULL DEFAULT 0,last_success INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(account,folder));
          CREATE TABLE IF NOT EXISTS drafts(account TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,updated_at INTEGER NOT NULL,PRIMARY KEY(account,id));
          CREATE TABLE IF NOT EXISTS outbox(account TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,state TEXT NOT NULL,send_at INTEGER NOT NULL,created_at INTEGER NOT NULL,updated_at INTEGER NOT NULL,error TEXT,PRIMARY KEY(account,id));
          CREATE INDEX IF NOT EXISTS due_outbox ON outbox(state,send_at);
          CREATE TABLE IF NOT EXISTS reminders(account TEXT NOT NULL,message_id TEXT NOT NULL,kind TEXT NOT NULL,due INTEGER NOT NULL,note TEXT NOT NULL DEFAULT '',fired INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(account,message_id,kind));
          CREATE TABLE IF NOT EXISTS local_rules(account TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(account,id));
          CREATE TABLE IF NOT EXISTS rule_runs(account TEXT NOT NULL,rule_id TEXT NOT NULL,message_id TEXT NOT NULL,PRIMARY KEY(account,rule_id,message_id));
          CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,json TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS private_preferences(account TEXT PRIMARY KEY,json TEXT NOT NULL);
          CREATE TABLE IF NOT EXISTS archives(account TEXT NOT NULL,id TEXT NOT NULL,json TEXT NOT NULL,PRIMARY KEY(account,id));
          PRAGMA user_version=1;").map_err(sql_error)?;
        // A crashed in-flight send is ambiguous. Never send it again automatically.
        connection.execute("UPDATE outbox SET state='uncertain',error=NULL,updated_at=? WHERE state='sending'",[now()]).map_err(sql_error)?;
        Ok(Self{connection:Mutex::new(connection),crypto})
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_,Connection>> { self.connection.lock().map_err(|_|"The local mail database is busy. Restart Yas Mail.".into()) }
    fn seal(&self,kind:&str,account:&str,id:&str,value:&Value)->Result<String>{self.crypto.seal(&context(kind,account,id),&value.to_string())}
    fn decode(&self,kind:&str,account:&str,id:&str,value:String)->Result<Value>{let plain=self.crypto.open_sealed(&context(kind,account,id),&value)?;serde_json::from_str(&plain).map_err(|_|"Saved mail data could not be read.".into())}
    fn blind(&self,account:&str,field:&str,value:&str)->String{self.crypto.blind(&context("index",account,field),value)}
    fn words(value:&str)->Vec<String>{value.split(|c:char|!c.is_alphanumeric()).filter(|v|!v.is_empty()).take(10_000).map(str::to_lowercase).collect()}
    fn index(&self,account:&str,field:&str,value:&str)->String{Self::words(value).into_iter().map(|word|self.blind(account,field,&word)).collect::<HashSet<_>>().into_iter().collect::<Vec<_>>().join(" ")}

    pub fn cache_page(&self, account:&str, folder:&str, items:&[Value], replace_complete:bool) -> Result<Vec<Value>> {
        valid_id(account)?;valid_id(folder)?;
        let mut db=self.lock()?;let tx=db.transaction().map_err(sql_error)?;
        let already_seeded:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sync_state WHERE account=?1 AND folder=?2)",params![account,folder],|r|r.get(0)).map_err(sql_error)?;
        if replace_complete {tx.execute("DELETE FROM memberships WHERE account=?1 AND folder=?2",params![account,folder]).map_err(sql_error)?;}
        let mut new=vec![];
        for item in items.iter().take(1000) {
            let id=item["id"].as_str().ok_or("A provider message has no identifier")?;valid_id(id)?;
            let serialized=item.to_string();if serialized.len()>MAX_MESSAGE {continue;}
            let existed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE account=?1 AND id=?2)",params![account,id],|r|r.get(0)).map_err(sql_error)?;
            let (subject,sender,recipients,body)=(item["subject"].as_str().unwrap_or(""),item["from"].as_str().unwrap_or(""),item["to"].as_str().unwrap_or(""),item["body"].as_str().unwrap_or(""));
            let sort_at=item["date"].as_str().and_then(|v|chrono::DateTime::parse_from_rfc3339(v).ok()).map(|v|v.timestamp()).unwrap_or(0);
            tx.execute("INSERT INTO messages(account,id,subject,sender,recipients,body,unread,starred,sort_at,json,updated_at) VALUES(?1,?2,'','','','',?3,?4,?5,?6,?7) ON CONFLICT(account,id) DO UPDATE SET unread=excluded.unread,starred=excluded.starred,sort_at=excluded.sort_at,json=excluded.json,updated_at=excluded.updated_at",params![account,id,item["unread"].as_bool().unwrap_or(false),item["starred"].as_bool().unwrap_or(false),sort_at,self.seal("message",account,id,item)?,now()]).map_err(sql_error)?;
            tx.execute("DELETE FROM message_fts WHERE account=?1 AND id=?2",params![account,id]).map_err(sql_error)?;
            tx.execute("INSERT INTO message_fts(account,id,subject,sender,recipients,body) VALUES(?1,?2,?3,?4,?5,?6)",params![account,id,self.index(account,"subject",subject),self.index(account,"from",sender),self.index(account,"to",recipients),self.index(account,"all",&format!("{subject} {sender} {recipients} {body}"))]).map_err(sql_error)?;
            tx.execute("INSERT OR IGNORE INTO memberships(account,id,folder) VALUES(?1,?2,?3)",params![account,id,folder]).map_err(sql_error)?;
            tx.execute("DELETE FROM attachments WHERE account=?1 AND message_id=?2",params![account,id]).map_err(sql_error)?;
            if let Some(files)=item["attachments"].as_array(){for file in files.iter().take(1000){if let Some(file_id)=file["id"].as_str(){tx.execute("INSERT OR REPLACE INTO attachments(account,message_id,id,name,size,json) VALUES(?1,?2,?3,'',?4,?5)",params![account,id,file_id,file["size"].as_i64(),self.seal("attachment",account,&format!("{}:{id}:{file_id}",id.len()),file)?]).map_err(sql_error)?;}}}
            if !existed&&already_seeded&&item["unread"].as_bool().unwrap_or(false){new.push(item.clone());}
        }
        tx.execute("INSERT INTO sync_state(account,folder,last_success) VALUES(?1,?2,?3) ON CONFLICT(account,folder) DO UPDATE SET last_success=excluded.last_success",params![account,folder,now()]).map_err(sql_error)?;
        tx.commit().map_err(sql_error)?;
        Ok(new)
    }
    pub fn list(&self, accounts:&[String], folder:&str, query:&str, offset:usize) -> Result<Value> {
        if accounts.is_empty(){return Ok(json!({"items":[],"cursor":null,"cached":true}));}
        if offset>1_000_000||query.len()>2000{return Err("The mail search or page offset is too large.".into());}
        let db=self.lock()?;
        let terms=search_terms(query);
        let searching=!terms.is_empty();
        let anywhere=["search","snoozed","reminders"].contains(&folder);
        let mut sql=format!("SELECT DISTINCT m.account,m.id,m.json FROM messages m {} WHERE m.account IN ({})",if searching{"JOIN message_fts f ON f.account=m.account AND f.id=m.id"}else{""},placeholders(accounts.len()));
        let mut values=accounts.iter().map(|a|rusqlite::types::Value::Text(a.clone())).collect::<Vec<_>>();
        if !anywhere{sql.push_str(" AND EXISTS(SELECT 1 FROM memberships v WHERE v.account=m.account AND v.id=m.id AND v.folder=?)");values.push(folder.to_string().into());}
        if folder=="snoozed"||folder=="reminders"{sql.push_str(" AND EXISTS(SELECT 1 FROM reminders r WHERE r.account=m.account AND r.message_id=m.id AND r.kind=? AND r.due>?)");values.push(if folder=="snoozed"{"snooze"}else{"followup"}.to_string().into());values.push(now().into());}
        let mut indexed=vec![];
        for term in terms{
            match term.split_once(':'){
                Some(("has","attachment"))=>sql.push_str(" AND EXISTS(SELECT 1 FROM attachments a WHERE a.account=m.account AND a.message_id=m.id)"),
                Some(("is","unread"))=>sql.push_str(" AND m.unread=1"),
                Some(("is","starred"))=>sql.push_str(" AND m.starred=1"),
                Some(("in",value))=>{sql.push_str(" AND EXISTS(SELECT 1 FROM memberships q WHERE q.account=m.account AND q.id=m.id AND q.folder=?)");values.push(value.to_string().into());},
                Some((field,value))if["after","before"].contains(&field)=>{let date=chrono::NaiveDate::parse_from_str(value,"%Y-%m-%d").map_err(|_|"Use YYYY-MM-DD for search dates.")?.and_hms_opt(0,0,0).unwrap().and_utc().timestamp();sql.push_str(if field=="after"{" AND m.sort_at>=?"}else{" AND m.sort_at<?"});values.push(date.into());},
                Some((field,value))if["from","to","subject"].contains(&field)=>{for word in Self::words(value){indexed.push((field.to_string(),word));}},
                _=>{for word in Self::words(&term){indexed.push(("all".to_string(),word));}}
            }
        }
        if !indexed.is_empty(){
            let alternatives=accounts.iter().map(|account|format!("({})",indexed.iter().map(|(field,word)|format!("\"{}\"",self.blind(account,field,word))).collect::<Vec<_>>().join(" AND "))).collect::<Vec<_>>().join(" OR ");
            sql.push_str(" AND message_fts MATCH ?");values.push(alternatives.into());
        }
        if folder!="snoozed"{sql.push_str(" AND NOT EXISTS(SELECT 1 FROM reminders s WHERE s.account=m.account AND s.message_id=m.id AND s.kind='snooze' AND s.due>?)");values.push(now().into());}
        sql.push_str(" ORDER BY m.sort_at DESC,m.account,m.id LIMIT ? OFFSET ?");values.push(((PAGE_SIZE+1)as i64).into());values.push((offset as i64).into());
        let mut stmt=db.prepare(&sql).map_err(sql_error)?;
        let rows=stmt.query_map(params_from_iter(values),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).map_err(sql_error)?;
        let mut items=vec![];
        for row in rows{let(account,id,raw)=row.map_err(sql_error)?;let mut item=self.decode("message",&account,&id,raw)?;item["accountId"]=json!(account);items.push(item);}
        let more=items.len()>PAGE_SIZE;items.truncate(PAGE_SIZE);
        let last_sync:Option<i64>=db.query_row(&format!("SELECT MAX(last_success) FROM sync_state WHERE account IN ({})",placeholders(accounts.len())),params_from_iter(accounts),|r|r.get(0)).map_err(sql_error)?;
        Ok(json!({"items":items,"cursor":if more{json!((offset+PAGE_SIZE).to_string())}else{Value::Null},"cached":true,"lastSync":last_sync}))
    }
    pub fn message(&self,account:&str,id:&str)->Result<Option<Value>>{
        self.lock()?.query_row("SELECT json FROM messages WHERE account=?1 AND id=?2",params![account,id],|r|r.get::<_,String>(0)).optional().map_err(sql_error)?.map(|raw|self.decode("message",account,id,raw)).transpose()
    }
    pub fn has_folder(&self,account:&str,folder:&str)->Result<bool>{self.lock()?.query_row("SELECT EXISTS(SELECT 1 FROM sync_state WHERE account=?1 AND folder=?2)",params![account,folder],|r|r.get(0)).map_err(sql_error)}
    pub fn backfill(&self,account:&str,folder:&str)->Result<(Option<String>,bool)>{
        let (raw,complete):(Option<String>,bool)=self.lock()?.query_row("SELECT cursor,complete FROM sync_state WHERE account=?1 AND folder=?2",params![account,folder],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql_error)?.unwrap_or((None,false));
        Ok((raw.map(|raw|self.decode("cursor",account,folder,raw).map(|v|v.as_str().unwrap_or("").to_string())).transpose()?,complete))
    }
    pub fn set_backfill(&self,account:&str,folder:&str,cursor:Option<&str>)->Result<()>{
        let raw=cursor.map(|value|self.seal("cursor",account,folder,&json!(value))).transpose()?;
        self.lock()?.execute("INSERT INTO sync_state(account,folder,cursor,complete,last_success) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(account,folder) DO UPDATE SET cursor=excluded.cursor,complete=excluded.complete,last_success=excluded.last_success",params![account,folder,raw,cursor.is_none(),now()]).map_err(sql_error)?;Ok(())
    }
    pub fn apply_action(&self,account:&str,id:&str,action:&str)->Result<()>{
        let Some(mut item)=self.message(account,id)? else{return Ok(())};
        match action{"read"=>item["unread"]=json!(false),"unread"=>item["unread"]=json!(true),"star"=>item["starred"]=json!(true),"unstar"=>item["starred"]=json!(false),"archive"|"trash"=>{},_=>return Ok(())}
        let db=self.lock()?;
        db.execute("UPDATE messages SET json=?1,unread=?2,starred=?3,updated_at=?4 WHERE account=?5 AND id=?6",params![self.seal("message",account,id,&item)?,item["unread"].as_bool().unwrap_or(false),item["starred"].as_bool().unwrap_or(false),now(),account,id]).map_err(sql_error)?;
        if action=="archive"||action=="trash"{db.execute("DELETE FROM memberships WHERE account=?1 AND id=?2 AND folder='inbox'",params![account,id]).map_err(sql_error)?;db.execute("INSERT OR IGNORE INTO memberships(account,id,folder)VALUES(?1,?2,?3)",params![account,id,if action=="archive"{"archive"}else{"trash"}]).map_err(sql_error)?;}
        Ok(())
    }
    pub fn save_draft(&self,account:&str,id:Option<&str>,draft:&Value)->Result<Value>{
        valid_id(account)?;let id=id.map(str::to_owned).unwrap_or_else(key);valid_id(&id)?;
        if draft.to_string().len()>MAX_DRAFT{return Err("The local draft exceeds the 30 MB storage limit.".into());}
        let raw=self.seal("draft",account,&id,draft)?;let db=self.lock()?;
        let count:i64=db.query_row("SELECT COUNT(*) FROM drafts WHERE account=?1",[account],|r|r.get(0)).map_err(sql_error)?;
        if count>=100&&!db.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE account=?1 AND id=?2)",params![account,id],|r|r.get::<_,bool>(0)).map_err(sql_error)?{return Err("This account has 100 local drafts. Remove an unused draft first.".into());}
        db.execute("INSERT INTO drafts(account,id,json,updated_at)VALUES(?1,?2,?3,?4)ON CONFLICT(account,id)DO UPDATE SET json=excluded.json,updated_at=excluded.updated_at",params![account,id,raw,now()]).map_err(sql_error)?;
        Ok(json!({"id":id,"accountId":account,"draft":draft,"updatedAt":now()}))
    }
    pub fn drafts(&self,accounts:&[String])->Result<Vec<Value>>{
        if accounts.is_empty(){return Ok(vec![])}
        let db=self.lock()?;let mut statement=db.prepare(&format!("SELECT account,id,json,updated_at FROM drafts WHERE account IN ({}) ORDER BY updated_at DESC LIMIT 200",placeholders(accounts.len()))).map_err(sql_error)?;
        let rows=statement.query_map(params_from_iter(accounts),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?))).map_err(sql_error)?;
        let mut out=vec![];for row in rows{let(account,id,raw,updated)=row.map_err(sql_error)?;out.push(json!({"accountId":account,"id":id,"draft":self.decode("draft",&account,&id,raw)?,"updatedAt":updated}));}Ok(out)
    }
    pub fn delete_draft(&self,account:&str,id:&str)->Result<()>{self.lock()?.execute("DELETE FROM drafts WHERE account=?1 AND id=?2",params![account,id]).map_err(sql_error)?;Ok(())}
    pub fn queue(&self,account:&str,draft:&Value,send_at:i64)->Result<Value>{
        valid_id(account)?;if send_at<now()-5||send_at>now()+366*86400{return Err("Choose a send time within the next year.".into())}
        if draft.to_string().len()>MAX_DRAFT{return Err("This scheduled message is too large to save.".into())}
        let id=key();let raw=self.seal("outbox",account,&id,draft)?;let db=self.lock()?;
        let queued:i64=db.query_row("SELECT COUNT(*) FROM outbox WHERE account=?1 AND state IN('queued','sending')",[account],|r|r.get(0)).map_err(sql_error)?;
        if queued>=200{return Err("This account already has 200 pending messages.".into())}
        db.execute("INSERT INTO outbox(account,id,json,state,send_at,created_at,updated_at)VALUES(?1,?2,?3,'queued',?4,?5,?5)",params![account,id,raw,send_at,now()]).map_err(sql_error)?;
        Ok(json!({"id":id,"accountId":account,"state":"queued","sendAt":send_at,"cancelUntil":send_at}))
    }
    pub fn jobs(&self,accounts:&[String])->Result<Vec<Value>>{
        if accounts.is_empty(){return Ok(vec![])}let db=self.lock()?;
        let mut stmt=db.prepare(&format!("SELECT account,id,state,send_at,json,error,created_at FROM outbox WHERE account IN ({}) ORDER BY created_at DESC LIMIT 200",placeholders(accounts.len()))).map_err(sql_error)?;
        let rows=stmt.query_map(params_from_iter(accounts),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?))).map_err(sql_error)?;
        let mut out=vec![];for row in rows{let(account,id,state,at,raw,error,created)=row.map_err(sql_error)?;let draft=self.decode("outbox",&account,&id,raw)?;let error=error.map(|v|self.decode("outbox-error",&account,&id,v)).transpose()?.unwrap_or_else(||if state=="uncertain"{json!("Sending was interrupted. Check your provider before sending again.")}else{Value::Null});out.push(json!({"accountId":account,"id":id,"state":state,"sendAt":at,"subject":draft["subject"],"to":draft["to"],"draft":draft,"error":error,"createdAt":created}));}Ok(out)
    }
    pub fn cancel_job(&self,account:&str,id:&str)->Result<bool>{Ok(self.lock()?.execute("UPDATE outbox SET state='cancelled',updated_at=?1 WHERE account=?2 AND id=?3 AND state='queued'",params![now(),account,id]).map_err(sql_error)?==1)}
    pub fn claim_due(&self)->Result<Option<Value>>{
        let mut db=self.lock()?;let tx=db.transaction().map_err(sql_error)?;
        let job=tx.query_row("SELECT account,id,json FROM outbox WHERE state='queued' AND send_at<=?1 ORDER BY send_at LIMIT 1",[now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?))).optional().map_err(sql_error)?;
        let Some((account,id,raw))=job else{return Ok(None)};
        let draft=self.decode("outbox",&account,&id,raw)?;
        tx.execute("UPDATE outbox SET state='sending',updated_at=?1 WHERE account=?2 AND id=?3 AND state='queued'",params![now(),account,id]).map_err(sql_error)?;tx.commit().map_err(sql_error)?;
        Ok(Some(json!({"accountId":account,"id":id,"draft":draft})))
    }
    pub fn finish_job(&self,account:&str,id:&str,success:bool,error:Option<&str>)->Result<()>{
        let error=error.map(|e|self.seal("outbox-error",account,id,&json!(e.chars().take(1000).collect::<String>()))).transpose()?;
        self.lock()?.execute("UPDATE outbox SET state=?1,error=?2,updated_at=?3 WHERE account=?4 AND id=?5 AND state='sending'",params![if success{"accepted"}else{"uncertain"},error,now(),account,id]).map_err(sql_error)?;Ok(())
    }
    pub fn set_reminder(&self,account:&str,id:&str,kind:&str,due:Option<i64>,note:&str)->Result<()>{
        valid_id(id)?;if !["snooze","followup"].contains(&kind)||note.len()>2000{return Err("Invalid reminder".into())}let db=self.lock()?;
        if let Some(due)=due{if due<=now()||due>now()+366*86400{return Err("Choose a reminder time within the next year.".into())}let note=self.seal("reminder",account,&format!("{}:{id}:{kind}",id.len()),&json!(note))?;db.execute("INSERT INTO reminders(account,message_id,kind,due,note,fired)VALUES(?1,?2,?3,?4,?5,0)ON CONFLICT(account,message_id,kind)DO UPDATE SET due=excluded.due,note=excluded.note,fired=0",params![account,id,kind,due,note]).map_err(sql_error)?;}
        else{db.execute("DELETE FROM reminders WHERE account=?1 AND message_id=?2 AND kind=?3",params![account,id,kind]).map_err(sql_error)?;}Ok(())
    }
    pub fn reminder_rows(&self,accounts:&[String],kind:Option<&str>)->Result<Vec<Value>>{
        if accounts.is_empty(){return Ok(vec![])}let db=self.lock()?;let mut sql=format!("SELECT account,message_id,kind,due,note,fired FROM reminders WHERE account IN ({})",placeholders(accounts.len()));let mut values=accounts.iter().map(|a|rusqlite::types::Value::Text(a.clone())).collect::<Vec<_>>();
        if let Some(kind)=kind{sql.push_str(" AND kind=?");values.push(kind.to_string().into());}sql.push_str(" ORDER BY due LIMIT 500");
        let mut stmt=db.prepare(&sql).map_err(sql_error)?;let rows=stmt.query_map(params_from_iter(values),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?))).map_err(sql_error)?;
        let mut out=vec![];for row in rows{let(account,id,kind,due,note,fired)=row.map_err(sql_error)?;let note=self.decode("reminder",&account,&format!("{}:{id}:{kind}",id.len()),note)?;out.push(json!({"accountId":account,"id":id,"kind":kind,"due":due,"note":note,"fired":fired}));}Ok(out)
    }
    pub fn due_reminders(&self)->Result<Vec<Value>>{
        let mut db=self.lock()?;let tx=db.transaction().map_err(sql_error)?;let mut out=vec![];
        {let mut stmt=tx.prepare("SELECT account,message_id,kind,due,note FROM reminders WHERE fired=0 AND due<=?1 ORDER BY due LIMIT 100").map_err(sql_error)?;
        let rows=stmt.query_map([now()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,i64>(3)?,r.get::<_,String>(4)?))).map_err(sql_error)?;
        for row in rows{let(account,id,kind,due,note)=row.map_err(sql_error)?;out.push(json!({"accountId":account,"id":id,"kind":kind,"due":due,"note":self.decode("reminder",&account,&format!("{}:{id}:{kind}",id.len()),note)?}));}}
        for row in &out{tx.execute("UPDATE reminders SET fired=1 WHERE account=?1 AND message_id=?2 AND kind=?3",params![row["accountId"].as_str(),row["id"].as_str(),row["kind"].as_str()]).map_err(sql_error)?;}tx.commit().map_err(sql_error)?;Ok(out)
    }
    pub fn attachment_search(&self,accounts:&[String],query:&str)->Result<Vec<Value>>{
        if accounts.is_empty(){return Ok(vec![])}let db=self.lock()?;
        let mut stmt=db.prepare(&format!("SELECT a.account,a.message_id,a.id,a.json,m.json FROM attachments a JOIN messages m ON m.account=a.account AND m.id=a.message_id WHERE a.account IN ({}) ORDER BY m.sort_at DESC LIMIT 5000",placeholders(accounts.len()))).map_err(sql_error)?;
        let rows=stmt.query_map(params_from_iter(accounts),|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?))).map_err(sql_error)?;
        let mut out=vec![];for row in rows{let(account,message,id,raw,mail)=row.map_err(sql_error)?;let mut item=self.decode("attachment",&account,&format!("{}:{message}:{id}",message.len()),raw)?;if !item["name"].as_str().unwrap_or("").to_lowercase().contains(&query.to_lowercase()){continue;}item["accountId"]=json!(account);item["messageId"]=json!(message);item["subject"]=self.decode("message",&account,&message,mail)?["subject"].clone();out.push(item);if out.len()>=200{break;}}Ok(out)
    }
    pub fn settings(&self,value:Option<&Value>)->Result<Value>{
        let db=self.lock()?;
        if let Some(value)=value{db.execute("INSERT INTO settings(key,json)VALUES('workspace',?1)ON CONFLICT(key)DO UPDATE SET json=excluded.json",[self.seal("settings","","workspace",value)?]).map_err(sql_error)?;}
        let raw:Option<String>=db.query_row("SELECT json FROM settings WHERE key='workspace'",[],|r|r.get(0)).optional().map_err(sql_error)?;
        Ok(raw.map(|raw|self.decode("settings","","workspace",raw)).transpose()?.unwrap_or_else(||json!({"syncSeconds":60,"backgroundSync":true,"notifications":false,"notificationPreview":false})))
    }
    pub fn preference(&self,account:&str)->Result<Option<Value>>{
        self.lock()?.query_row("SELECT json FROM private_preferences WHERE account=?1",[account],|r|r.get::<_,String>(0)).optional().map_err(sql_error)?.map(|raw|self.decode("preferences",account,"private",raw)).transpose()
    }
    pub fn save_preference(&self,account:&str,value:&Value)->Result<Value>{
        let Some(object)=value.as_object()else{return Err("Account preferences must be an object.".into())};
        if value.to_string().len()>2_000_000{return Err("Private account preferences exceed the 2 MB limit.".into())}
        for key in object.keys(){if !["signature","signatures","newSignature","replySignature","learnStyle"].contains(&key.as_str()){return Err("Unsupported private account preference.".into())}}
        if let Some(signatures)=value.get("signatures"){
            let signatures=signatures.as_array().ok_or("Signatures must be a list.")?;if signatures.len()>20{return Err("Keep at most 20 signatures per account.".into())}
            for signature in signatures{
                for (field,max) in [("id",100),("name",128),("html",500_000)]{let text=signature[field].as_str().ok_or("Each signature needs an ID, name and HTML.")?;if text.len()>max{return Err("A signature field exceeds its size limit.".into())}}
                let html=signature["html"].as_str().unwrap_or("");let mut rest=html;
                while let Some(at)=rest.find("data:image/"){rest=&rest[at..];let Some(base)=rest.find("base64,")else{break};if base>80{return Err("Invalid inline signature image.".into())}rest=&rest[base+7..];let size=rest.chars().take_while(|c|c.is_ascii_alphanumeric()||"+/=".contains(*c)).count();if size>266_668{return Err("Inline signature images may be at most 200 KB each.".into())}rest=&rest[size..];}
            }
        }
        if value.get("learnStyle").is_some_and(|v|!v.is_boolean()){return Err("Writing-style learning must be explicitly on or off.".into())}
        self.lock()?.execute("INSERT INTO private_preferences(account,json)VALUES(?1,?2)ON CONFLICT(account)DO UPDATE SET json=excluded.json",params![account,self.seal("preferences",account,"private",value)?]).map_err(sql_error)?;Ok(value.clone())
    }
    pub fn archive_message(&self,account:&str,id:&str,message:&Value,raw:&[u8])->Result<()>{
        use base64::Engine;
        if !id.starts_with("local:")||raw.len()>20_000_000{return Err("Local archive messages must have a local identifier and be at most 20 MB.".into())}
        let encoded=base64::engine::general_purpose::STANDARD.encode(raw);
        self.lock()?.execute("INSERT INTO archives(account,id,json)VALUES(?1,?2,?3)ON CONFLICT(account,id)DO UPDATE SET json=excluded.json",params![account,id,self.seal("archive",account,id,&json!(encoded))?]).map_err(sql_error)?;
        self.cache_page(account,"local-archive",&[message.clone()],false)?;Ok(())
    }
    pub fn archive_raw(&self,account:&str,id:&str)->Result<Option<Vec<u8>>>{
        use base64::Engine;
        let raw:Option<String>=self.lock()?.query_row("SELECT json FROM archives WHERE account=?1 AND id=?2",params![account,id],|r|r.get(0)).optional().map_err(sql_error)?;
        raw.map(|raw|{let value=self.decode("archive",account,id,raw)?;base64::engine::general_purpose::STANDARD.decode(value.as_str().ok_or("The local archive record is invalid.")?).map_err(|_|"The local archive record is invalid.".into())}).transpose()
    }
    pub fn local_rules(&self,account:&str)->Result<Vec<Value>>{
        let db=self.lock()?;let mut stmt=db.prepare("SELECT id,json FROM local_rules WHERE account=?1").map_err(sql_error)?;let rows=stmt.query_map([account],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).map_err(sql_error)?;
        let mut out=vec![];for row in rows{let(id,raw)=row.map_err(sql_error)?;out.push(self.decode("rule",account,&id,raw)?);}Ok(out)
    }
    pub fn save_rule(&self,account:&str,rule:&Value)->Result<Value>{
        let mut rule=rule.clone();let id=rule["id"].as_str().filter(|v|!v.is_empty()).map(str::to_owned).unwrap_or_else(key);valid_id(&id)?;rule["id"]=json!(id);
        if rule.to_string().len()>20_000{return Err("This rule is too large.".into())}
        self.lock()?.execute("INSERT INTO local_rules(account,id,json)VALUES(?1,?2,?3)ON CONFLICT(account,id)DO UPDATE SET json=excluded.json",params![account,id,self.seal("rule",account,&id,&rule)?]).map_err(sql_error)?;Ok(rule)
    }
    pub fn delete_rule(&self,account:&str,id:&str)->Result<()>{self.lock()?.execute("DELETE FROM local_rules WHERE account=?1 AND id=?2",params![account,id]).map_err(sql_error)?;Ok(())}
    pub fn claim_rule(&self,account:&str,rule_id:&str,message_id:&str)->Result<bool>{Ok(self.lock()?.execute("INSERT OR IGNORE INTO rule_runs(account,rule_id,message_id)VALUES(?1,?2,?3)",params![account,rule_id,message_id]).map_err(sql_error)?==1)}
    pub fn forget_account(&self,account:&str)->Result<()>{
        let mut db=self.lock()?;let tx=db.transaction().map_err(sql_error)?;
        for table in ["message_fts","messages","sync_state","drafts","outbox","reminders","local_rules","rule_runs","private_preferences","archives"]{tx.execute(&format!("DELETE FROM {table} WHERE account=?1"),[account]).map_err(sql_error)?;}tx.commit().map_err(sql_error)?;Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store()->MailStore{MailStore::from_connection(Connection::open_in_memory().unwrap(),CacheCrypto::ephemeral()).unwrap()}
    fn item(id:&str,subject:&str)->Value{json!({"id":id,"subject":subject,"from":"sender@example.com","to":"recipient@example.com","date":"2026-11-09T10:00:00Z","body":"Quarterly budget plain text","unread":true,"attachments":[{"id":"file1","name":"budget.pdf","size":100}]})}
    #[test]fn cache_and_search_are_account_isolated_and_fts_syntax_is_literal(){let s=store();s.cache_page("a","inbox",&[item("same","Alpha")],true).unwrap();s.cache_page("b","inbox",&[item("same","Beta")],true).unwrap();let v=s.list(&["a".into()],"inbox","budget",0).unwrap();assert_eq!(v["items"].as_array().unwrap().len(),1);assert_eq!(v["items"][0]["subject"],"Alpha");assert_eq!(v["items"][0]["accountId"],"a");assert!(s.list(&["a".into()],"search","\" OR *",0).is_ok());assert!(s.list(&[],"inbox","",0).unwrap()["items"].as_array().unwrap().is_empty());}
    #[test]fn initial_sync_does_not_notify_and_refresh_updates_memberships(){let s=store();assert!(s.cache_page("a","inbox",&[item("one","One")],true).unwrap().is_empty());assert_eq!(s.cache_page("a","inbox",&[item("one","One"),item("two","Two")],true).unwrap().len(),1);s.cache_page("a","inbox",&[item("two","Two")],true).unwrap();assert_eq!(s.list(&["a".into()],"inbox","",0).unwrap()["items"].as_array().unwrap().len(),1);assert!(s.message("a","one").unwrap().is_some());}
    #[test]fn local_drafts_preserve_rich_content_attachments_and_account_scope(){let s=store();let draft=json!({"to":"a@example.com","html":"<p>Rich draft</p>","files":[{"name":"file.txt","data":"YWJj"}]});let saved=s.save_draft("a",None,&draft).unwrap();assert_eq!(s.drafts(&["a".into()]).unwrap()[0]["draft"],draft);assert!(s.drafts(&["b".into()]).unwrap().is_empty());s.delete_draft("b",saved["id"].as_str().unwrap()).unwrap();assert_eq!(s.drafts(&["a".into()]).unwrap().len(),1);}
    #[test]fn cancelled_outbox_cannot_be_claimed_and_sending_is_never_retried(){let s=store();let q=s.queue("a",&json!({"subject":"cancel"}),now()+1).unwrap();assert!(!s.cancel_job("b",q["id"].as_str().unwrap()).unwrap());assert!(s.cancel_job("a",q["id"].as_str().unwrap()).unwrap());assert!(s.claim_due().unwrap().is_none());let q=s.queue("a",&json!({"subject":"send"}),now()).unwrap();assert!(s.claim_due().unwrap().is_some());assert!(s.claim_due().unwrap().is_none());assert!(!s.cancel_job("a",q["id"].as_str().unwrap()).unwrap());s.finish_job("a",q["id"].as_str().unwrap(),false,Some("ambiguous response")).unwrap();assert_eq!(s.jobs(&["a".into()]).unwrap().iter().find(|v|v["id"]==q["id"]).unwrap()["state"],"uncertain");assert!(s.claim_due().unwrap().is_none());}
    #[test]fn crash_recovery_marks_inflight_sends_uncertain(){let connection=Connection::open_in_memory().unwrap();let s=MailStore::from_connection(connection,CacheCrypto::ephemeral()).unwrap();s.queue("a",&json!({"subject":"test"}),now()).unwrap();s.claim_due().unwrap().unwrap();let db=s.connection.into_inner().unwrap();let reopened=MailStore::from_connection(db,s.crypto).unwrap();assert_eq!(reopened.jobs(&["a".into()]).unwrap()[0]["state"],"uncertain");assert!(reopened.claim_due().unwrap().is_none());}
    #[test]fn ciphertext_and_blind_index_never_store_mail_or_private_preferences_as_plaintext(){let path=std::env::temp_dir().join(format!("yas-mail-encrypted-test-{}.sqlite",key()));let s=MailStore::from_connection(Connection::open(&path).unwrap(),CacheCrypto::ephemeral()).unwrap();let secret="SensitiveUniqueNeedle928571";s.cache_page("account","inbox",&[item("message",secret)],true).unwrap();s.save_draft("account",None,&json!({"body":secret})).unwrap();s.queue("account",&json!({"subject":secret}),now()+60).unwrap();s.save_preference("account",&json!({"signature":secret,"learnStyle":false})).unwrap();assert_eq!(s.list(&["account".into()],"inbox",secret,0).unwrap()["items"].as_array().unwrap().len(),1);s.connection.lock().unwrap().execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();drop(s);let data=std::fs::read(&path).unwrap();assert!(!data.windows(secret.len()).any(|w|w==secret.as_bytes()));let _=std::fs::remove_file(&path);}
    #[test]fn ciphertext_cannot_be_reassigned_to_another_account_or_message(){let s=store();s.cache_page("a","inbox",&[item("one","Secret")],true).unwrap();let raw:String=s.connection.lock().unwrap().query_row("SELECT json FROM messages WHERE account='a' AND id='one'",[],|r|r.get(0)).unwrap();assert!(s.decode("message","b","one",raw.clone()).is_err());assert!(s.decode("message","a","two",raw).is_err());}
    #[test]fn snoozed_messages_disappear_without_touching_provider_and_attachments_are_scoped(){let s=store();s.cache_page("a","inbox",&[item("one","One")],true).unwrap();s.set_reminder("a","one","snooze",Some(now()+60),"").unwrap();assert!(s.list(&["a".into()],"inbox","",0).unwrap()["items"].as_array().unwrap().is_empty());assert_eq!(s.attachment_search(&["a".into()],"budget").unwrap().len(),1);assert!(s.attachment_search(&["b".into()],"budget").unwrap().is_empty());s.set_reminder("a","one","snooze",None,"").unwrap();assert_eq!(s.list(&["a".into()],"inbox","",0).unwrap()["items"].as_array().unwrap().len(),1);}
    #[test]fn rule_actions_are_claimed_once_and_disconnect_clears_local_data(){let s=store();s.cache_page("a","inbox",&[item("one","One")],true).unwrap();s.save_draft("a",None,&json!({"body":"secret"})).unwrap();assert!(s.claim_rule("a","r","one").unwrap());assert!(!s.claim_rule("a","r","one").unwrap());assert!(s.claim_rule("b","r","one").unwrap());s.forget_account("a").unwrap();assert!(s.message("a","one").unwrap().is_none());assert!(s.drafts(&["a".into()]).unwrap().is_empty());assert!(s.attachment_search(&["a".into()],"").unwrap().is_empty());}
}
