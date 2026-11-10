//! Application payload encryption. The random master key stays in the system credential store.
use crate::{err, Result};
use aes_gcm::{aead::{Aead,Payload},Aes256Gcm,KeyInit,Nonce};
use base64::{engine::general_purpose::{STANDARD,URL_SAFE_NO_PAD},Engine};
use hmac::{Hmac,Mac};
use rand::RngCore;
use sha2::{Digest,Sha256};
use std::{fs::OpenOptions,path::Path,time::{Duration,Instant}};

const MAX:usize=40_000_000;
pub struct CacheCrypto { cipher:Aes256Gcm,index_key:[u8;32] }
impl CacheCrypto {
    pub fn open(path:&Path)->Result<Self>{
        let parent=path.parent().ok_or("Invalid mail storage path.")?.canonicalize().map_err(err)?;
        let identity=parent.join(path.file_name().ok_or("Invalid mail storage path.")?).to_string_lossy().to_string();
        let identity=if cfg!(windows){identity.to_lowercase()}else{identity};
        let key_name=format!("cache-key-v1:{}",URL_SAFE_NO_PAD.encode(Sha256::digest(identity.as_bytes())));
        let lock=OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path.with_extension("key.lock")).map_err(err)?;
        let until=Instant::now()+Duration::from_secs(5);
        while fs2::FileExt::try_lock_exclusive(&lock).is_err(){if Instant::now()>until{return Err("Mail storage is busy. Close the other Yas Mail process and retry.".into());}std::thread::sleep(Duration::from_millis(25));}
        let entry=crate::entry(&key_name)?;
        let master=match entry.get_password(){
            Ok(encoded)=>{let key=STANDARD.decode(encoded).map_err(|_|"The saved mail encryption key is invalid. Restore its system credential backup.")?;if key.len()!=32{return Err("The saved mail encryption key is invalid.".into());}let mut master=[0;32];master.copy_from_slice(&key);master},
            Err(keyring::Error::NoEntry)=>{
                if std::fs::metadata(path).is_ok_and(|m|m.len()>0){return Err("The encryption key for existing mail storage is unavailable. Restore the original system credentials; the database has not been changed.".into());}
                let mut key=[0;32];rand::thread_rng().fill_bytes(&mut key);entry.set_password(&STANDARD.encode(key)).map_err(|_|"Could not protect the mail encryption key in the system credential store.")?;key
            },
            Err(_)=>return Err("the system credential store could not unlock mail storage.".into()),
        };
        Self::from_master(master)
    }
    fn from_master(mut master:[u8;32])->Result<Self>{
        let encryption=derive(&master,b"YasMail payload encryption v1");let index_key=derive(&master,b"YasMail blind search v1");master.fill(0);
        let cipher=Aes256Gcm::new_from_slice(&encryption).map_err(|_|"Could not initialize encrypted mail storage.")?;
        Ok(Self{cipher,index_key})
    }
    pub fn seal(&self,context:&str,plaintext:&str)->Result<String>{
        if plaintext.len()>MAX||context.len()>4096{return Err("Mail storage payload exceeds the limit.".into());}
        let mut nonce=[0;12];rand::thread_rng().fill_bytes(&mut nonce);
        let encrypted=self.cipher.encrypt(Nonce::from_slice(&nonce),Payload{msg:plaintext.as_bytes(),aad:context.as_bytes()}).map_err(|_|"Could not encrypt mail storage.")?;
        let mut data=Vec::with_capacity(12+encrypted.len());data.extend_from_slice(&nonce);data.extend_from_slice(&encrypted);Ok(format!("v1:{}",STANDARD.encode(data)))
    }
    pub fn open_sealed(&self,context:&str,ciphertext:&str)->Result<String>{
        if ciphertext.len()>54_000_000||context.len()>4096{return Err("Invalid encrypted mail payload.".into());}
        let encoded=ciphertext.strip_prefix("v1:").ok_or("Unknown encrypted mail format. The data has not been changed.")?;
        let data=STANDARD.decode(encoded).map_err(|_|"Encrypted mail data is invalid.")?;
        if data.len()<28{return Err("Encrypted mail data is incomplete.".into());}
        let bytes=self.cipher.decrypt(Nonce::from_slice(&data[..12]),Payload{msg:&data[12..],aad:context.as_bytes()}).map_err(|_|"Mail storage authentication failed. The data or its context has changed.")?;
        String::from_utf8(bytes).map_err(|_|"Decrypted mail data is invalid.".into())
    }
    pub fn blind(&self,context:&str,token:&str)->String{
        let mut mac=<Hmac<Sha256> as Mac>::new_from_slice(&self.index_key).expect("fixed HMAC key");mac.update(context.as_bytes());mac.update(&[0]);mac.update(token.as_bytes());URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    }
    #[cfg(test)]
    pub fn ephemeral()->Self{let mut key=[0;32];rand::thread_rng().fill_bytes(&mut key);Self::from_master(key).unwrap()}
}
fn derive(key:&[u8;32],context:&[u8])->[u8;32]{let mut mac=<Hmac<Sha256> as Mac>::new_from_slice(key).expect("fixed HMAC key");mac.update(context);mac.finalize().into_bytes().into()}
impl Drop for CacheCrypto {fn drop(&mut self){self.index_key.fill(0);}}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn randomized_encryption_and_row_context_integrity(){let crypto=CacheCrypto::ephemeral();let text="private synthetic message and recipient";let a=crypto.seal("account-A/message-1",text).unwrap();let b=crypto.seal("account-A/message-1",text).unwrap();assert_ne!(a,b);assert!(!a.contains(text));assert_eq!(crypto.open_sealed("account-A/message-1",&a).unwrap(),text);assert!(crypto.open_sealed("account-B/message-1",&a).is_err());let mut raw=STANDARD.decode(a.strip_prefix("v1:").unwrap()).unwrap();*raw.last_mut().unwrap()^=1;assert!(crypto.open_sealed("account-A/message-1",&format!("v1:{}",STANDARD.encode(raw))).is_err());}
    #[test]fn blind_tokens_are_scoped_and_do_not_expose_search_terms(){let crypto=CacheCrypto::ephemeral();assert_eq!(crypto.blind("account-A","invoice"),crypto.blind("account-A","invoice"));assert_ne!(crypto.blind("account-A","invoice"),crypto.blind("account-B","invoice"));assert!(!crypto.blind("account-A","invoice").contains("invoice"));}
}
