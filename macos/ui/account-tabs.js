const palette=['#2563eb','#7c3aed','#0f766e','#be185d','#b45309','#334155','#0e7490','#4f46e5'];
const escape=value=>String(value??'').replace(/[&<>"']/g,character=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));
const appearance=(prefs,id)=>Object.hasOwn(prefs||{},id)&&prefs[id]&&typeof prefs[id]==='object'?prefs[id]:{};
function darkColor(value){
  if(typeof value!=='string'||!/^#[\da-f]{6}$/i.test(value))return null;
  const channels=[1,3,5].map(offset=>{const c=parseInt(value.slice(offset,offset+2),16)/255;return c<=.04045?c/12.92:((c+.055)/1.055)**2.4});
  const luminance=channels[0]*.2126+channels[1]*.7152+channels[2]*.0722;
  return 1.05/(luminance+.05)>=4.5?value.toLowerCase():null;
}
function colorIndex(id){let hash=2166136261;for(const character of String(id||'')){hash^=character.codePointAt(0);hash=Math.imul(hash,16777619)}return (hash>>>0)%palette.length}
export function accountColor(account,prefs={}){
  return darkColor(appearance(prefs,account?.id).color)||(account?.id==='demo'?palette[0]:palette[colorIndex(account?.id)]);
}
export function ensureAccountAppearance(accounts,prefs){
  const used=new Set(accounts.map(account=>darkColor(appearance(prefs,account.id).color)).filter(Boolean));
  let changed=false;
  for(const account of accounts){
    const saved=appearance(prefs,account.id);
    if(darkColor(saved.color))continue;
    const start=accounts.indexOf(account)%palette.length;
    // Prefer an unused color, then cycle once every palette color has been used.
    let assigned=palette[start];
    for(let offset=0;offset<palette.length;offset++){
      const candidate=palette[(start+offset)%palette.length];
      if(!used.has(candidate)){assigned=candidate;break}
    }
    Object.defineProperty(prefs,account.id,{value:{...saved,color:assigned},writable:true,enumerable:true,configurable:true});
    used.add(assigned);changed=true;
  }
  return changed;
}
function avatarInitials(email){
  const local=String(email||'').split('@')[0],parts=local.split(/[\s._+\-]+/).filter(Boolean);
  if(parts.length>1)return (parts[0][0]+parts[1][0]).toUpperCase();
  return (parts[0]||'?').slice(0,2).toUpperCase();
}
export function accountAvatar(account,prefs={}){
  return String(appearance(prefs,account?.id).avatar||(account?.id==='demo'?'YM':avatarInitials(account?.email)));
}
export function renderAccountTabs(accounts,activeId,prefs={}){
  const entries=accounts.length?accounts:[{id:'demo',email:'Demo workspace',provider:'demo'}];
  const selected=entries.some(account=>account.id===activeId)?activeId:entries[0].id;
  return `<div class="account-tabs" role="tablist" aria-label="Email accounts">${entries.map(account=>{
    const isDemo=account.id==='demo'&&account.provider==='demo',label=isDemo?'Demo workspace':account.email;
    const provider=isDemo?'Demo':account.provider==='gmail'?'Google':account.provider==='outlook'?'Microsoft':account.provider==='imap'?'IMAP':account.provider==='pop3'?'POP3':account.provider==='smtp'?'SMTP · send only':'Email account';
    const avatar=accountAvatar(account,prefs);
    const active=account.id===selected;
    return `<button type="button" class="account-tab${active?' active':''}" role="tab" data-account-tab="${escape(account.id)}" aria-label="${escape(label)}" aria-selected="${active}" aria-controls="account-workspace" tabindex="${active?'0':'-1'}" title="${escape(label)}" style="--account-color:${accountColor(account,prefs)}" data-tauri-drag-region="false"><span class="account-tab-avatar" aria-hidden="true">${escape(avatar)}</span><span class="account-tab-copy"><span class="account-tab-label">${escape(label)}</span><span class="account-tab-provider">${provider}</span></span></button>`;
  }).join('')}</div>`;
}
export function nextAccountTab(accounts,currentId,key){
  const ids=accounts.length?accounts.map(account=>account.id):['demo'];
  const current=ids.indexOf(currentId);
  switch(key){
    case 'Home':return ids[0];
    case 'End':return ids.at(-1);
    case 'ArrowRight':return ids[current<0?0:(current+1)%ids.length];
    case 'ArrowLeft':return ids[current<0?ids.length-1:(current-1+ids.length)%ids.length];
    default:return null;
  }
}
