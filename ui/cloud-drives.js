import {oauthClientIdError,usesPublisherSignIn,providerSignInReady} from './oauth-setup.js';
export const cloudProviders = Object.freeze({
  google: {name:'Google Drive',badge:'GD',help:'Continue with Google in your browser to give Yas Mail read access to your files. Choose the Google account you want to connect.',secret:''},
  onedrive: {name:'OneDrive',badge:'OD',help:'Continue with Microsoft in your browser to give Yas Mail read access to your files. Personal and work accounts can be connected when enabled by your provider.',secret:''},
  dropbox: {name:'Dropbox',badge:'DB',help:'Use your Dropbox app key with account_info.read, files.metadata.read, files.content.read and sharing.read. Register http://localhost:53682/callback as its redirect URI.',secret:''},
  zoho: {name:'Zoho WorkDrive',badge:'ZW',help:'Use a Zoho OAuth desktop client in your account’s data centre. Register http://localhost:53683/callback and grant the read scopes listed in CLOUD-DRIVES.md.',secret:'Zoho client secret',secretRequired:true}
});
const zohoRegions=[['us','United States / .com'],['eu','Europe'],['in','India'],['au','Australia'],['jp','Japan'],['ca','Canada'],['cn','China'],['sa','Saudi Arabia'],['ae','United Arab Emirates']];
const hostMatches=(host,domain)=>host===domain||host.endsWith('.'+domain);
export function safeCloudLink(value,provider){
  if(typeof value!=='string'||value.length>8192||/[\u0000-\u0020\u007f]/.test(value))return null;
  try{
    const url=new URL(value),host=url.hostname.toLowerCase();
    if(url.protocol!=='https:'||url.username||url.password||(url.port&&url.port!=='443'))return null;
    const allowed=provider==='google'?['drive.google.com','docs.google.com']:
      provider==='onedrive'?['onedrive.live.com','1drv.ms','sharepoint.com','sharepoint-df.com']:
      provider==='dropbox'?['dropbox.com']:
      provider==='zoho'?['zoho.com','zoho.eu','zoho.in','zoho.com.au','zoho.jp','zoho.com.cn','zoho.sa','zoho.ae','zohocloud.ca','zohoapis.com','zohoapis.eu','zohoapis.in','zohoapis.com.au','zohoapis.jp','zohoapis.ca','zohoapis.com.cn','zohoapis.sa','zohoapis.ae']:[];
    return allowed.some(domain=>hostMatches(host,domain))?url.href:null;
  }catch{return null}
}
export const attachmentLimit=provider=>provider==='outlook'?3_000_000:9_000_000;
export function attachmentByteLength(file){
  const data=file?.data;
  if(typeof data!=='string')throw Error('The attachment data is not valid base64.');
  const padding=data.endsWith('==')?2:data.endsWith('=')?1:0;
  if(data.length%4!==0||/[^A-Za-z0-9+/=]/.test(data)||data.indexOf('=')!==(padding?data.length-padding:-1))throw Error('The attachment data is not valid base64.');
  return data.length/4*3-padding;
}
export function appendAttachment(existing,incoming,provider){
  if(!incoming||typeof incoming.name!=='string'||!incoming.name.trim())throw Error('The attachment has no filename.');
  if(/[\u0000-\u001f\u007f]/.test(incoming.name))throw Error('The attachment filename contains unsupported control characters.');
  const size=attachmentByteLength(incoming),limit=attachmentLimit(provider);
  if(existing.reduce((sum,file)=>sum+attachmentByteLength(file),0)+size>limit)throw Error(`Attachments exceed ${limit/1_000_000} MB total. Insert an existing cloud link instead, or remove an attachment.`);
  return [...existing,{...incoming,name:incoming.name.trim(),size}];
}
export function formatCloudSize(size){
  if(!Number.isFinite(size)||size<0)return 'Size unavailable';
  if(size<1000)return `${Math.round(size)} B`;
  if(size<1_000_000)return `${(size/1000).toFixed(size<10_000?1:0)} KB`;
  return `${(size/1_000_000).toFixed(size<10_000_000?1:0)} MB`;
}
export function cloudLinkText(item,url){return `${String(item.name||'Cloud file').replace(/[\r\n\u0000-\u001f\u007f]+/g,' ').trim()}: ${url}`}

// Connection credentials live only in the current form. Native Rust owns OAuth,
// refresh tokens, bounded downloads and provider API calls.
export function createCloudDrives({native,invoke,$,esc,button,toast,active,canAttach,getFiles,setFiles,renderAttachments}){
  let connections=[],connectionError='',connectBusy=false,disconnectConfirm=null,connectionRevision=0;
  let picker=null,revision=0,formVisible=false,authProviders=null,authLoading=null,authError='';
  const jobs=new Set();
  const providerInfo=value=>cloudProviders[value]||{name:String(value||'Cloud drive'),badge:'☁'};
  const currentDraft=()=>{const dialog=$('#modal'),body=$('#body');return dialog?.open&&body&&dialog._composeToken?{dialog,body,token:dialog._composeToken,owner:active().id}:null};
  const sameDraft=draft=>{const current=currentDraft();return !!current&&current.token===draft.token&&current.body===draft.body&&current.owner===draft.owner};
  const connected=()=>connections.find(connection=>connection.id===picker?.connection);
  function pending(){const draft=currentDraft();return !!draft&&[...jobs].some(job=>job.token===draft.token)}
  async function refresh(){
    const ticket=++connectionRevision;connectionError='';
    if(!native){connections=[];return connections}
    try{const result=await invoke('cloud_connections');if(ticket===connectionRevision)connections=Array.isArray(result)?result:[]}
    catch(error){if(ticket===connectionRevision)connectionError=String(error)}
    return connections;
  }
  const providerReady=provider=>!usesPublisherSignIn(provider)||providerSignInReady(authProviders,provider);
  const availability=provider=>!usesPublisherSignIn(provider)?'':!native?'Connect this drive in the Windows app.':authProviders===null?'Checking sign-in availability…':providerReady(provider)?'':authError||'Sign-in for this provider is unavailable in this preview build. You can explore the demo while sign-in is being prepared.';
  async function ensureProviderStatus(){
    if(authError&&!authLoading){authProviders=null;authError=''}
    if(!native||authProviders!==null)return;
    if(!authLoading)authLoading=invoke('oauth_providers').then(value=>{authProviders=Array.isArray(value)?value:[]}).catch(()=>{authProviders=[];authError='Sign-in availability could not be checked. Close this form and try again.'}).finally(()=>{authLoading=null});
    await authLoading;
  }
  function providerDetails(provider){
    const info=providerInfo(provider),manual=!usesPublisherSignIn(provider);
    return `<p class="muted cloud-setup-help">${esc(info.help||'')}</p>${manual?'<label class="field">Application client ID / app key<input name="clientId" autocomplete="off" placeholder="Your registered application client ID / app key"></label>':''}${info.secret?`<label class="field">${esc(info.secret)}<input name="clientSecret" type="password" autocomplete="off" ${info.secretRequired?'required':''}></label>`:''}${provider==='zoho'?`<label class="field">Zoho data centre<select name="region" aria-label="Zoho data centre">${zohoRegions.map(([id,label])=>`<option value="${id}">${esc(label)}</option>`).join('')}</select></label>`:''}`;
  }
  function connectForm(context){
    return `<div class="cloud-connect-form" data-cloud-form="${context}"><h4>Connect a cloud drive</h4><label class="field">Cloud provider<select name="provider" data-cloud-provider aria-label="Cloud provider" ${connectBusy||native&&authProviders===null?'disabled':''}>${Object.entries(cloudProviders).map(([id,info])=>`<option value="${id}">${info.name}</option>`).join('')}</select></label><div data-cloud-provider-details>${providerDetails('google')}</div><p class="muted">Sign-in opens in your browser. Your password stays with your provider, and Windows protects your connection tokens.</p><div class="cloud-connect-status" data-cloud-connect-status role="status">${connectBusy?'Waiting for browser sign-in…':availability('google')}</div><div class="cloud-actions">${button(connectBusy?'Waiting for sign-in…':'Continue in browser','cloud-connect','primary',`data-cloud-connect-submit ${!native||connectBusy||!providerReady('google')?'disabled':''}`)}${button('Cancel','cloud-hide-connect','quiet',connectBusy?'disabled':'')}</div></div>`;
  }
  function connectionsMarkup(){
    if(connectionError)return `<div class="error" role="alert">${esc(connectionError)}</div>`;
    if(!connections.length)return `<p class="muted">No cloud drives connected. Add Google Drive, OneDrive, Dropbox, or Zoho WorkDrive.</p>`;
    return connections.map(connection=>{
      const info=providerInfo(connection.provider),confirm=disconnectConfirm===connection.id;
      return `<div class="cloud-connection"><span class="cloud-provider-badge" aria-hidden="true">${esc(info.badge)}</span><div class="grow"><strong>${esc(connection.label||info.name)}</strong><div class="muted">${esc(info.name)}${connection.email?' · '+esc(connection.email):''}</div>${confirm?'<p class="muted">Disconnect this drive from Yas Mail? Your files and existing sharing permissions are kept.</p>':''}</div>${confirm?`${button('Keep connected','cloud-keep','quiet')}${button('Disconnect drive','cloud-disconnect-confirm','quiet danger',`data-connection="${esc(connection.id)}"`)}`:button('Disconnect','cloud-disconnect','quiet',`data-connection="${esc(connection.id)}"`)}</div>`;
    }).join('');
  }
  function settingsContent(){return `${!native?'<div class="notice">Open the Windows app to connect real cloud accounts. The browser demo does not connect providers.</div>':''}${connectionsMarkup()}<div class="cloud-actions">${button('＋ Connect cloud drive','cloud-show-connect','primary')}${button('Refresh connections','cloud-refresh-connections')}</div>${formVisible?connectForm('settings'):''}`}
  function settingsMarkup(){return `<section class="settings-card" id="cloud-settings-card"><h3>Cloud drives</h3><p class="muted">Bring files into your email from your drives. Attach a copy or insert an existing link; Yas Mail does not create public links or change access permissions.</p><div data-cloud-settings-content>${settingsContent()}</div></section>`}
  function paintSettings(){const mount=$('[data-cloud-settings-content]');if(mount)mount.innerHTML=settingsContent()}
  function paintPicker(){
    const host=$('#cloud-panel');if(!host||!picker||!sameDraft(picker.draft))return;
    const connection=connected(),info=providerInfo(connection?.provider),busy=picker.loading||pending();
    host.hidden=false;
    host.innerHTML=`<div class="cloud-picker-head"><div><strong>Attach from cloud</strong><div class="muted">Your message stays open while you browse.</div></div>${button('✕','cloud-close','quiet','aria-label="Close cloud picker"')}</div>${!native?'<div class="notice">Connect drives in the Windows app. No cloud files are simulated in this demo.</div>':''}${connectionError?`<div class="error" role="alert">${esc(connectionError)}</div>`:''}${connections.length?`<div class="cloud-picker-toolbar"><label class="field">Drive<select data-cloud-connection aria-label="Cloud drive" ${busy?'disabled':''}>${connections.map(c=>`<option value="${esc(c.id)}" ${c.id===picker.connection?'selected':''}>${esc(c.label||providerInfo(c.provider).name)} · ${esc(providerInfo(c.provider).name)}</option>`).join('')}</select></label>${button('＋ Connect another','cloud-show-connect','quiet',connectBusy?'disabled':'')}</div>`:`<p class="muted">No drives connected. You can connect one here without closing your draft.</p>${button('＋ Connect a drive','cloud-show-connect','primary',connectBusy?'disabled':'')}`}${picker.showForm?connectForm('compose'):''}${connection?`<div class="cloud-folder-toolbar">${button('← Back','cloud-back','quiet',!picker.history.length||busy?'disabled':'')}${button('Root','cloud-root','quiet',busy?'disabled':'')}<span class="cloud-folder-path" title="${esc([info.name,...picker.history.map(p=>p.name),picker.folderName].filter(Boolean).join(' / '))}">${esc(picker.folderName||info.name)}</span></div><div class="cloud-search"><input data-cloud-search aria-label="Search cloud files" placeholder="Search files…" value="${esc(picker.query)}" ${busy?'disabled':''}>${button('Search','cloud-search','quiet',busy?'disabled':'')}${button('↻','cloud-reload','quiet',`aria-label="Refresh cloud files" ${busy?'disabled':''}`)}</div><p class="cloud-access-note muted">Links keep the provider’s existing access rules. A recipient may need permission. Online documents can be linked when a download is unavailable.</p>${picker.error?`<div class="error" role="alert">${esc(picker.error)}</div>`:''}${picker.loading?'<div class="cloud-status" role="status">Loading files…</div>':''}<div class="cloud-file-list" role="list" aria-label="Cloud files">${picker.items.map((item,index)=>itemMarkup(item,index,connection,busy)).join('')}${!picker.items.length&&!picker.loading&&!picker.error?'<div class="cloud-status">No files found.</div>':''}</div>${picker.cursor?button('Load more files','cloud-more','quiet',busy?'disabled':''):''}`:''}${pending()?'<p class="cloud-status" role="status">Preparing your cloud file… You can keep editing your message.</p>':''}`;
  }
  function itemMarkup(item,index,connection,busy){
    const folder=item.isFolder===true,disabled=busy?'disabled':'',size=folder?'Folder':formatCloudSize(item.size);
    const available=canAttach()&&item.canDownload===true&&(!Number.isFinite(item.size)||item.size<=attachmentLimit(active().provider));
    return `<div class="cloud-file" role="listitem"><span class="cloud-file-icon" aria-hidden="true">${folder?'▰':'▤'}</span><div class="cloud-file-info">${folder?button(esc(item.name),'cloud-folder','quiet cloud-folder-name',`data-index="${index}" ${disabled}`):`<strong title="${esc(item.name)}">${esc(item.name)}</strong>`}<span class="muted">${esc(size)}${!folder&&!item.canDownload?' · Link only':''}</span></div><div class="cloud-file-actions">${folder?button('Open','cloud-folder','quiet',`data-index="${index}" ${disabled}`):`${button('Insert link','cloud-link','quiet',`data-index="${index}" ${disabled}`)}${button('Attach copy','cloud-attach','quiet',`data-index="${index}" ${disabled||!available?'disabled':''} title="${esc(!canAttach()?'Connect an email account to attach files':!item.canDownload?'This online file is available as a link':!available?'This file exceeds the attachment limit':'Download a copy into this message')}"`)}`}</div></div>`;
  }
  async function listFiles(append=false){
    if(!picker||!connected())return;
    const state=picker,ticket=++revision,connection=state.connection,requestedCursor=append?state.cursor:null;
    state.loading=true;state.error='';paintPicker();
    try{
      const page=await invoke('cloud_list',{connection,folder:state.folder,query:state.query,cursor:requestedCursor});
      if(picker!==state||revision!==ticket||!sameDraft(state.draft))return;
      const items=Array.isArray(page.items)?page.items:[];
      const merged=append?[...state.items,...items]:items,seen=new Set();
      state.items=merged.filter(item=>item&&typeof item.id==='string'&&!seen.has(item.id)&&seen.add(item.id));
      state.cursor=typeof page.cursor==='string'&&page.cursor&&page.cursor!==requestedCursor?page.cursor:null;
    }catch(error){if(picker===state&&revision===ticket)state.error=String(error)}
    finally{if(picker===state&&revision===ticket){state.loading=false;paintPicker()}}
  }
  async function open(){
    const draft=currentDraft();if(!draft)return;
    picker={draft,connection:null,folder:null,folderName:'',history:[],query:'',items:[],cursor:null,error:'',loading:true,showForm:false};
    const state=picker;paintPicker();await refresh();
    if(picker!==state||!sameDraft(draft))return;
    state.loading=false;state.connection=connections[0]?.id||null;paintPicker();if(state.connection)await listFiles();
  }
  function close(){++revision;picker=null;const host=$('#cloud-panel');if(host){host.hidden=true;host.innerHTML=''}}
  async function connect(el){
    if(connectBusy)return;
    const form=el.closest('[data-cloud-form]');if(!form)return;
    const provider=form.querySelector('[name=provider]').value,clientId=form.querySelector('[name=clientId]')?.value.trim()||'',clientSecret=form.querySelector('[name=clientSecret]')?.value||'',region=form.querySelector('[name=region]')?.value||'us';
    const status=form.querySelector('[data-cloud-connect-status]');
    if(!native){status.textContent='Open the Windows app to connect a cloud drive.';return}
    if(usesPublisherSignIn(provider)&&!providerReady(provider)){status.textContent=availability(provider);return}
    const setupError=usesPublisherSignIn(provider)?'':oauthClientIdError(provider,clientId);if(setupError){status.textContent=setupError;form.querySelector('[name=clientId]').focus();return}
    if(cloudProviders[provider]?.secretRequired&&!clientSecret){status.textContent='Enter the Zoho client secret for your registered application.';return}
    let succeeded=false;connectBusy=true;form.querySelectorAll('button,input,select').forEach(control=>control.disabled=true);status.textContent='Waiting for browser sign-in… Finish in your browser. This request times out if sign-in is not completed.';
    try{
      const args={provider};if(!usesPublisherSignIn(provider))Object.assign(args,{clientId,clientSecret});if(provider==='zoho')args.region=region;
      const connection=await invoke('cloud_connect',args);await refresh();succeeded=true;
      formVisible=false;disconnectConfirm=null;
      if(picker&&sameDraft(picker.draft)){picker.showForm=false;picker.connection=connection.id;picker.folder=null;picker.folderName='';picker.history=[];picker.query='';picker.items=[];picker.cursor=null;await listFiles()}
      paintSettings();toast('Cloud drive connected');
    }catch(error){if(status.isConnected)status.textContent=String(error);else toast(String(error))}
    finally{connectBusy=false;if(form.isConnected){form.querySelectorAll('button,input,select').forEach(control=>control.disabled=false);form.querySelector('[data-cloud-connect-submit]').disabled=!native||!providerReady(provider)}if(succeeded)paintPicker()}
  }
  async function useFile(index,mode){
    if(!picker||pending())return;
    const state=picker,connection=connected(),item=state.items[index],draft=state.draft;
    if(!connection||!item||item.isFolder||!sameDraft(draft))return;
    const job={token:draft.token};jobs.add(job);state.error='';paintPicker();
    const original=draft.body.value,start=draft.body.selectionStart,end=draft.body.selectionEnd;
    try{
      if(mode==='attach'){
        if(!canAttach()||!item.canDownload)throw Error('This file cannot be attached. Insert an existing link instead.');
        if(Number.isFinite(item.size)&&getFiles().reduce((sum,file)=>sum+attachmentByteLength(file),0)+item.size>attachmentLimit(active().provider))throw Error('This file exceeds the remaining attachment allowance. Insert an existing link instead.');
        const file=await invoke('cloud_download',{connection:connection.id,item:item.id,mailAccount:draft.owner});
        if(!sameDraft(draft)){toast('The draft was closed or changed. The cloud file was not attached.');return}
        setFiles(appendAttachment(getFiles(),{...file,source:providerInfo(connection.provider).name},active().provider));renderAttachments();toast('Cloud file attached to this message');
      }else{
        const result=await invoke('cloud_link',{connection:connection.id,item:item.id});
        const url=safeCloudLink(result?.webUrl,connection.provider);if(!url)throw Error('The provider did not return a supported secure file link. Manage the file in your provider, or attach a copy.');
        if(!sameDraft(draft)){toast('The draft was closed or changed. The cloud link was not inserted.');return}
        const text=cloudLinkText(item,url),unchanged=draft.body.value===original;
        const at=unchanged?start:draft.body.value.length,until=unchanged?end:at;
        const before=draft.body.value.slice(0,at),prefix=before&&!before.endsWith('\n')?'\n':'';
        draft.body.setSelectionRange(at,until);draft.body.setRangeText(prefix+text+'\n',at,until,'end');draft.body.dispatchEvent(new Event('input',{bubbles:true}));
        toast(unchanged?'Existing cloud link inserted. Check recipient access before sending.':'Your draft changed; the cloud link was added at the end.');
      }
    }catch(error){if(picker===state&&sameDraft(draft))state.error=String(error);else toast(String(error))}
    finally{jobs.delete(job);if(picker===state)paintPicker()}
  }
  async function action(name,el){
    if(!name.startsWith('cloud-'))return false;
    if(name==='cloud-open'){await open();return true}
    if(name==='cloud-close'){close();return true}
    if(name==='cloud-show-connect'){if(connectBusy)return true;if(el.closest('#cloud-panel')){if(picker)picker.showForm=true;paintPicker()}else{formVisible=true;paintSettings()}await ensureProviderStatus();paintSettings();paintPicker();return true}
    if(name==='cloud-hide-connect'){if(el.closest('#cloud-panel')){if(picker)picker.showForm=false;paintPicker()}else{formVisible=false;paintSettings()}return true}
    if(name==='cloud-connect'){await connect(el);return true}
    if(name==='cloud-refresh-connections'){await refresh();paintSettings();return true}
    if(name==='cloud-disconnect'){disconnectConfirm=el.dataset.connection;paintSettings();return true}
    if(name==='cloud-keep'){disconnectConfirm=null;paintSettings();return true}
    if(name==='cloud-disconnect-confirm'){
      el.disabled=true;try{await invoke('cloud_disconnect',{connection:el.dataset.connection});disconnectConfirm=null;await refresh();paintSettings();toast('Cloud drive disconnected. Provider files were kept.')}catch(error){toast(String(error));el.disabled=false}return true;
    }
    if(!picker)return true;
    if(name==='cloud-folder'){
      const item=picker.items[Number(el.dataset.index)];if(!item?.isFolder)return true;
      picker.history.push({folder:picker.folder,name:picker.folderName,query:picker.query});picker.folder=item.id;picker.folderName=item.name;picker.query='';picker.items=[];picker.cursor=null;await listFiles();return true;
    }
    if(name==='cloud-back'){const previous=picker.history.pop();if(previous){picker.folder=previous.folder;picker.folderName=previous.name;picker.query=previous.query;picker.items=[];picker.cursor=null;await listFiles()}return true}
    if(name==='cloud-root'){picker.folder=null;picker.folderName='';picker.history=[];picker.query='';picker.items=[];picker.cursor=null;await listFiles();return true}
    if(name==='cloud-search'){picker.query=$('[data-cloud-search]')?.value.trim()||'';picker.items=[];picker.cursor=null;await listFiles();return true}
    if(name==='cloud-reload'||name==='cloud-more'){await listFiles(name==='cloud-more');return true}
    if(name==='cloud-link'||name==='cloud-attach'){await useFile(Number(el.dataset.index),name==='cloud-attach'?'attach':'link');return true}
    return true;
  }
  document.addEventListener('change',event=>{
    if(event.target.matches('[data-cloud-provider]')){
      const form=event.target.closest('[data-cloud-form]');form.querySelector('[data-cloud-provider-details]').innerHTML=providerDetails(event.target.value);form.querySelector('[data-cloud-connect-status]').textContent=availability(event.target.value);form.querySelector('[data-cloud-connect-submit]').disabled=!native||connectBusy||!providerReady(event.target.value);
    }
    if(event.target.matches('[data-cloud-connection]')&&picker){picker.connection=event.target.value;picker.folder=null;picker.folderName='';picker.history=[];picker.query='';picker.items=[];picker.cursor=null;listFiles()}
  });
  document.addEventListener('keydown',event=>{if(event.target.matches('[data-cloud-search]')&&event.key==='Enter'){event.preventDefault();action('cloud-search',event.target).catch(error=>toast(String(error)))}});
  $('#modal').addEventListener('close',close);
  return {settingsMarkup,refresh,action,close,hasPending:pending};
}
