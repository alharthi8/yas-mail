export function parseCsv(input){
 const rows=[];let row=[],value='',quoted=false;input=input.replace(/^\uFEFF/,'');
 for(let i=0;i<input.length;i++){const c=input[i];if(quoted){if(c==='"'&&input[i+1]==='"'){value+='"';i++}else if(c==='"')quoted=false;else value+=c}else if(c==='"'){if(value)throw Error('Invalid quote in CSV');quoted=true}else if(c===','){row.push(value);value=''}else if(c==='\r'||c==='\n'){if(c==='\r'&&input[i+1]==='\n')i++;row.push(value);if(row.some(Boolean))rows.push(row);row=[];value=''}else value+=c}
 if(quoted)throw Error('The CSV contains an unclosed quoted field.');row.push(value);if(row.some(Boolean))rows.push(row);return rows;
}
const unescapeV=s=>s.replace(/\\n/gi,'\n').replace(/\\([,;\\])/g,'$1');
export function parseContacts(input,filename){
 let contacts=[];
 if(/\.vcf$/i.test(filename)||input.trimStart().startsWith('BEGIN:VCARD')){
  if(/ENCODING=(QUOTED-PRINTABLE|B)/i.test(input))throw Error('Encoded vCard 2.1 is not supported. Export UTF-8 vCard 3/4 or CSV.');
  input=input.replace(/\r?\n[ \t]/g,'');
  for(const card of input.split(/BEGIN:VCARD/i).slice(1)){if(!/END:VCARD/i.test(card))throw Error('An incomplete vCard was found.');let c={name:'',email:'',phone:''};for(const line of card.split(/\r?\n/)){const idx=line.indexOf(':');if(idx<0)continue;const key=line.slice(0,idx).split(';')[0].split('.').at(-1).toUpperCase(),v=unescapeV(line.slice(idx+1)).trim();if(key==='FN')c.name=v;if(key==='N'&&!c.name)c.name=v.split(';').filter(Boolean).reverse().join(' ');if(key==='EMAIL'&&!c.email)c.email=v;if(key==='TEL'&&!c.phone)c.phone=v.replace(/^tel:/,'');}if(c.name||c.email)contacts.push(c);}
 }else{
  const rows=parseCsv(input);if(rows.length<2)throw Error('CSV needs a header row and at least one contact.');const headers=rows.shift().map(h=>h.toLowerCase().replace(/[^a-z0-9]/g,''));const column=(names)=>headers.findIndex(h=>names.includes(h));const name=column(['name','fullname','displayname']),first=column(['firstname','givenname']),last=column(['lastname','familyname']),email=column(['email','emailaddress','email1value','email1address','emailaddress1']),phone=column(['phone','phonenumber','mobilephone','phone1value','primaryphone','mobile']);
  if([name,first,email].every(i=>i<0))throw Error('No recognized columns. Use Name, Email, Phone headers.');
  contacts=rows.map(r=>({name:(r[name]||[r[first],r[last]].filter(Boolean).join(' ')||r[email]||'').trim(),email:(r[email]||'').trim(),phone:(r[phone]||'').trim()})).filter(c=>c.name||c.email);
 }
 if(!contacts.length)throw Error('No contacts found.');if(contacts.length>500)throw Error('Import up to 500 contacts at a time.');for(const c of contacts)if(c.email&&!/^[^\s<>@]+@[^\s<>@]+$/.test(c.email))throw Error('Invalid email in import: '+c.email);return contacts;
}
export function contactKey(c){return c.email?'email:'+c.email.toLowerCase():'person:'+String(c.name).toLowerCase()+':'+(c.phone||'')}
export function exportContacts(contacts,format){
 if(format==='vcf'){const escape=s=>String(s||'').replace(/\\/g,'\\\\').replace(/\n/g,'\\n').replace(/,/g,'\\,').replace(/;/g,'\\;').replace(/\r/g,'');return contacts.map(c=>['BEGIN:VCARD','VERSION:3.0','FN:'+escape(c.name),'EMAIL:'+escape(c.email),'TEL:'+escape(c.phone),'END:VCARD'].join('\r\n')).join('\r\n')+'\r\n'}
 const quote=s=>{s=String(s||'');if(/^[=+\-@\t\r]/.test(s))s="'"+s;return '"'+s.replace(/"/g,'""')+'"'};return '\uFEFF'+[['Name','Email','Phone'],...contacts.map(c=>[c.name,c.email,c.phone])].map(r=>r.map(quote).join(',')).join('\r\n');
}
