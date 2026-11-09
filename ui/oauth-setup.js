const microsoftId=/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const googleId=/^[a-z0-9][a-z0-9_-]*\.apps\.googleusercontent\.com$/i;

export function oauthClientIdError(provider,value){
  const id=String(value??'').trim();
  if(!id)return 'Enter the application client ID from your provider’s app registration.';
  if(new TextEncoder().encode(id).length>4096)return 'The application client ID is too long. Copy only the ID from your app registration.';
  if(provider==='outlook'||provider==='onedrive'){
    if(!microsoftId.test(id))return 'Enter Microsoft’s Application (client) ID from Entra → App registrations → Overview. It has the form xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx. Your email address is entered later on Microsoft’s sign-in page.';
  }else if(provider==='gmail'||provider==='google'){
    if(!googleId.test(id))return 'Enter the Desktop app OAuth client ID from Google Cloud → APIs & Services → Credentials. It ends in .apps.googleusercontent.com. Your email address is entered later on Google’s sign-in page.';
  }else if(id.includes('@')||/\s|[\u0000-\u001f\u007f-\u009f]/.test(id)||id.includes('://')){
    return 'Enter your registered application client ID / app key. Your email address is entered later on the provider’s sign-in page.';
  }
  return '';
}

const escapeText=value=>String(value??'').replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
const signInProviders=[{provider:'gmail',name:'Google',detail:'Gmail & Google Workspace'},{provider:'outlook',name:'Microsoft',detail:'Outlook, Microsoft 365 & Exchange Online'}];
export const usesPublisherSignIn=provider=>['gmail','outlook','google','onedrive'].includes(provider);
export function normalizeSignInProviders(value){
  return signInProviders.map(info=>({...info,configured:Array.isArray(value)&&value.some(item=>item?.provider===info.provider&&item.configured===true)}));
}
export function providerSignInReady(value,provider){
  const id=provider==='google'?'gmail':provider==='onedrive'?'outlook':provider;
  return normalizeSignInProviders(value).some(item=>item.provider===id&&item.configured);
}
export function providerMark(provider){
  return provider==='outlook'?'<span class="sign-in-mark microsoft-mark" aria-hidden="true"><i></i><i></i><i></i><i></i></span>':'<span class="sign-in-mark google-mark" aria-hidden="true">G</span>';
}
export function accountSignInMarkup(value,{loading=false,native=true,error='',notice=''}={}){
  const providers=normalizeSignInProviders(value),unavailable=!loading&&native&&providers.some(info=>!info.configured);
  return `<div class="sign-in-intro"><span class="sign-in-app-icon" aria-hidden="true"><img src="logo.svg" alt=""></span><h3>Your mail. One clear place.</h3><p>Choose Google or Microsoft for browser sign-in, or connect another email server manually.</p></div><div class="sign-in-providers">${providers.map(info=>`<button class="sign-in-provider" data-action="sign-in" data-provider="${info.provider}" ${loading||!native||!info.configured?'disabled':''}>${providerMark(info.provider)}<span><strong>Continue with ${info.name}</strong><small>${info.detail}${loading?' · Checking availability…':!native?' · Available in the Windows app':info.configured?'':' · Unavailable in this preview build'}</small></span><span class="sign-in-arrow" aria-hidden="true">→</span></button>`).join('')}<button class="sign-in-provider manual-provider" data-action="manual-open" ${!native?'disabled':''}><span class="sign-in-mark manual-provider-mark" aria-hidden="true">＠</span><span><strong>Other email / manual setup</strong><small>IMAP, POP3, SMTP &amp; compatible Exchange servers</small></span><span class="sign-in-arrow" aria-hidden="true">→</span></button></div>${unavailable?'<p class="sign-in-availability" role="status">Sign-in isn’t enabled for every provider in this preview build. You can explore the demo while sign-in is being prepared.</p>':''}${!native?'<p class="sign-in-availability" role="status">Open the Windows app to connect your accounts. You can explore the demo here.</p>':''}${notice?`<p class="sign-in-availability" role="status">${escapeText(notice)}</p>`:''}${error?`<p class="error sign-in-error" role="alert">${escapeText(error)}</p>`:''}<p class="sign-in-privacy"><span aria-hidden="true">◇</span> Google and Microsoft passwords stay with your provider. Manual connections protect saved credentials in Windows. Your accounts stay separate.</p><div class="modal-footer sign-in-footer"><button class="quiet" data-action="close">${native?'Back to Yas Mail':'Explore demo'}</button></div>`;
}
export function createAccountSignIn({native,invoke,$,modal,onConnected}){
  let session=null,busy=false;
  const current=state=>session===state&&state.host.isConnected&&$('#modal')?.open;
  function paint(state,error='',notice=''){
    if(current(state))state.host.innerHTML=accountSignInMarkup(state.providers,{loading:state.loading,native,error,notice});
  }
  async function open(){
    if(busy)return;
    modal('Add your email','<div data-sign-in-content></div>');
    const state={host:$('[data-sign-in-content]'),providers:[],loading:native};session=state;paint(state);
    if(!native)return;
    try{const providers=await invoke('oauth_providers');if(!current(state))return;state.providers=providers;state.loading=false;paint(state)}
    catch{if(!current(state))return;state.loading=false;paint(state,'Sign-in availability could not be checked. Close this window and try again.')}
  }
  async function connect(provider){
    const state=session;if(busy||!state||!current(state)||!native||!providerSignInReady(state.providers,provider)||!['gmail','outlook'].includes(provider))return;
    busy=true;state.cancelling=false;state.cancelPromise=null;state.cancelAcknowledged=false;
    const controls=[...$('#modal').querySelectorAll('button')];controls.forEach(control=>control.disabled=true);
    state.host.innerHTML=`<div class="sign-in-pending" role="status" aria-live="polite"><div class="sign-in-orbit">${providerMark(provider)}<span class="sign-in-spinner" aria-hidden="true"></span></div><h3>Finish signing in in your browser</h3><p>Yas Mail is waiting for ${provider==='gmail'?'Google':'Microsoft'} to confirm your account.</p><span class="sign-in-waiting">You can return here when you’re done.</span><small>This request times out automatically if sign-in is not completed.</small><div class="modal-footer sign-in-footer"><button class="quiet" data-action="cancel-sign-in">Cancel sign-in</button></div></div>`;
    try{const account=await invoke('connect',{provider});if(current(state))await onConnected(account)}
    catch(error){if(state.cancelPromise)await state.cancelPromise.catch(()=>{});if(current(state))paint(state,state.cancelAcknowledged?'':String(error),state.cancelAcknowledged?'Sign-in cancelled.':'')}
    finally{busy=false;controls.forEach(control=>{if(control.isConnected)control.disabled=false})}
  }
  async function cancel(){
    const state=session;if(!busy||!state||!current(state)||state.cancelling)return;
    state.cancelling=true;const control=state.host.querySelector('[data-action="cancel-sign-in"]'),status=state.host.querySelector('.sign-in-waiting');if(control)control.disabled=true;if(status)status.textContent='Cancelling sign-in…';
    state.cancelPromise=invoke('cancel_sign_in').then(cancelled=>{state.cancelAcknowledged=cancelled===true;if(current(state)&&busy&&status)status.textContent=cancelled?'Cancelling sign-in…':'Finishing sign-in…'}).catch(error=>{state.cancelling=false;if(current(state)&&busy){if(control)control.disabled=false;if(status)status.textContent='Could not cancel yet. You can try again or finish in your browser.'}throw error});
    try{await state.cancelPromise}catch{}
  }
  $('#modal').addEventListener('cancel',event=>{if(busy&&session&&current(session))event.preventDefault()});
  return {open,connect,cancel,isPending:()=>busy};
}
