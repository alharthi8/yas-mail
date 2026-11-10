//! An app-owned, cloud-disabled Ollama process. Never attaches to an arbitrary shared endpoint.
use crate::{err,Result};
use std::{path::PathBuf,process::{Child,Command,Stdio},sync::{Mutex,OnceLock}};
use tokio::sync::Mutex as AsyncMutex;
struct Runtime {child:Child,port:u16,#[cfg(windows)]job:isize}
static RUNTIME:OnceLock<Mutex<Option<Runtime>>>=OnceLock::new();
static START:OnceLock<AsyncMutex<()>>=OnceLock::new();
fn state()->&'static Mutex<Option<Runtime>>{RUNTIME.get_or_init(||Mutex::new(None))}
fn executable()->Result<PathBuf>{
    let mut paths=Vec::new();
    #[cfg(windows)] {
        if let Some(local)=std::env::var_os("LOCALAPPDATA"){paths.push(PathBuf::from(local).join("Programs/Ollama/ollama.exe"));}
        if let Some(programs)=std::env::var_os("ProgramFiles"){paths.push(PathBuf::from(programs).join("Ollama/ollama.exe"));}
    }
    #[cfg(target_os="macos")] {
        // Finder launches do not inherit the terminal PATH. Use known installation locations.
        paths.extend(["/Applications/Ollama.app/Contents/Resources/ollama", "/opt/homebrew/bin/ollama", "/usr/local/bin/ollama"].map(PathBuf::from));
        if let Some(home)=std::env::var_os("HOME"){paths.push(PathBuf::from(home).join("Applications/Ollama.app/Contents/Resources/ollama"));}
    }
    paths.into_iter().find(|p|p.is_file()).ok_or("Install Ollama for your operating system to enable local AI. Yas Mail does not use a cloud fallback.".into())
}
#[cfg(windows)]
fn protect_process(child:&Child)->Result<isize>{
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation::CloseHandle,System::JobObjects::*};
    // Only our newly-created server and its future model workers enter this job.
    unsafe {
        let job=CreateJobObjectW(std::ptr::null(),std::ptr::null());
        if job.is_null(){return Err("Could not isolate the local AI process.".into());}
        let mut info:JOBOBJECT_EXTENDED_LIMIT_INFORMATION=std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags=JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(job,JobObjectExtendedLimitInformation,&info as *const _ as *const _,std::mem::size_of_val(&info) as u32)==0 || AssignProcessToJobObject(job,child.as_raw_handle() as _)==0 {CloseHandle(job);return Err("Could not isolate the local AI process. No email was sent to it.".into());}
        Ok(job as isize)
    }
}
impl Drop for Runtime{fn drop(&mut self){
    #[cfg(target_os="macos")] unsafe { libc::kill(-(self.child.id() as i32), libc::SIGKILL); }
let _=self.child.kill();let _=self.child.wait();#[cfg(windows)]unsafe{windows_sys::Win32::Foundation::CloseHandle(self.job as _);}}}
pub fn stop(){if let Ok(mut runtime)=state().lock(){runtime.take();}}
fn running()->Option<u16>{let mut runtime=state().lock().ok()?;if let Some(value)=runtime.as_mut(){if matches!(value.child.try_wait(),Ok(None)){return Some(value.port);}}runtime.take();None}
pub async fn endpoint()->Result<String>{
    let _start=START.get_or_init(||AsyncMutex::new(())).lock().await;
    if let Some(port)=running(){return Ok(format!("http://127.0.0.1:{port}"));}
    let path=executable()?;
    let listener=std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST,0)).map_err(err)?;
    let port=listener.local_addr().map_err(err)?.port();drop(listener);
    let mut command=Command::new(path);
    command.arg("serve").env("OLLAMA_HOST",format!("127.0.0.1:{port}")).env("OLLAMA_NO_CLOUD","1").env("OLLAMA_NOHISTORY","1").env("OLLAMA_NOPRUNE","1").env("OLLAMA_DEBUG","false").env("OLLAMA_ORIGINS","").env("OLLAMA_MAX_LOADED_MODELS","1").env("OLLAMA_NUM_PARALLEL","1").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    // Do not inherit a remote host, API key, web proxy, or alternate model directory.
    for key in ["OLLAMA_API_KEY","OLLAMA_REMOTES","OLLAMA_MODELS","HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","http_proxy","https_proxy","all_proxy"]{command.env_remove(key);}
    #[cfg(windows)]{use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
    #[cfg(target_os="macos")] {use std::os::unix::process::CommandExt;command.process_group(0);}
    let mut child=command.spawn().map_err(|_|"Could not start the private local AI runtime.")?;
    #[cfg(windows)]let job=match protect_process(&child){Ok(job)=>job,Err(error)=>{let _=child.kill();let _=child.wait();return Err(error)}};
    *state().lock().map_err(err)?=Some(Runtime{child,port,#[cfg(windows)]job});
    let endpoint=format!("http://127.0.0.1:{port}");
    let client=reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).timeout(std::time::Duration::from_secs(2)).build().map_err(err)?;
    for _ in 0..40 {
        if running()!=Some(port){return Err("The private local AI runtime could not start.".into());}
        if let Ok(response)=client.get(format!("{endpoint}/api/status")).send().await{if response.status().is_success(){if let Ok(value)=response.json::<serde_json::Value>().await{if value["cloud"]["disabled"]==true{return Ok(endpoint)}else{stop();return Err("Local AI refused: the private runtime did not confirm cloud access is disabled.".into());}}}}
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    stop();Err("The private local AI runtime did not become ready. Update Ollama and retry.".into())
}
