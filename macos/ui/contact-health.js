export function scanContacts(contacts){
 const issues=new Map(),indexes=new Map(),groups=[];
 for(const c of contacts){const name=String(c.name||'').trim(),email=String(c.email||'').trim(),phone=String(c.phone||'').trim(),digits=phone.replace(/\D/g,'').replace(/^00/,'');const flags=[];
  if(!name)flags.push('missing-name');if(!phone)flags.push('missing-phone');if(!email)flags.push('missing-email');
  if(email&&!/^[^\s<>@]+@[^\s<>@]+\.[^\s<>@]+$/.test(email))flags.push('invalid-email');
  if(phone&&(digits.length<7||digits.length>15))flags.push('review-phone');
  issues.set(c.id,flags);
  for(const key of [email?'email:'+email.toLowerCase():null,digits.length>=7?'phone:'+digits:null].filter(Boolean)){if(!indexes.has(key))indexes.set(key,[]);indexes.get(key).push(c.id)}
 }
 const unique=new Set();for(const [key,ids]of indexes){if(ids.length<2)continue;const signature=[...ids].sort().join('|');if(unique.has(signature))continue;unique.add(signature);groups.push({key,ids});for(const id of ids)if(!issues.get(id).includes('duplicate'))issues.get(id).push('duplicate');}
 return {issues,groups,counts:Object.fromEntries(['missing-name','missing-phone','missing-email','invalid-email','review-phone','duplicate'].map(key=>[key,[...issues.values()].filter(flags=>flags.includes(key)).length]))};
}
export function suggestName(email){const local=String(email||'').split('@')[0];if(!local||/^\d+$/.test(local))return '';return local.replace(/[._+\-]+/g,' ').replace(/\d+/g,'').trim().replace(/\b\w/g,c=>c.toUpperCase())}
export const issueNames={'missing-name':'Missing name','missing-phone':'Missing phone','missing-email':'Missing email','invalid-email':'Invalid email','review-phone':'Review phone','duplicate':'Duplicate match'};
export function healthMarkup(report,selected,button){return `<div class="contact-health"><div class="section-title"><div><strong>Contact health</strong><div class="muted">Review missing details and matching primary email addresses or phone numbers.</div></div>${button('Clean formatting','clean-contacts')}</div><div class="health-filters">${button('All contacts','health-filter',selected==='all'?'primary':'quiet','data-filter="all"')}${Object.entries(issueNames).map(([key,label])=>button(label+' · '+report.counts[key],'health-filter',selected===key?'primary':'quiet',`data-filter="${key}"`)).join('')}</div></div>`}
