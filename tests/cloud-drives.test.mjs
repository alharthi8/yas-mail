import test from 'node:test';
import assert from 'node:assert/strict';
import {safeCloudLink,attachmentByteLength,attachmentLimit,appendAttachment,formatCloudSize,cloudLinkText,createCloudDrives} from '../ui/cloud-drives.js';
const file=(name,text)=>({name,data:Buffer.from(text).toString('base64')});

test('cloud links require the selected provider and safe HTTPS boundaries',()=>{
  assert.equal(safeCloudLink('https://drive.google.com/file/d/123/view','google'),'https://drive.google.com/file/d/123/view');
  assert.equal(safeCloudLink('https://tenant.sharepoint.com/personal/file','onedrive'),'https://tenant.sharepoint.com/personal/file');
  assert.equal(safeCloudLink('https://tenant.sharepoint-df.com/personal/file','onedrive'),'https://tenant.sharepoint-df.com/personal/file');
  assert.equal(safeCloudLink('https://www.dropbox.com/scl/fi/abc/report.pdf?rlkey=123','dropbox'),'https://www.dropbox.com/scl/fi/abc/report.pdf?rlkey=123');
  assert.equal(safeCloudLink('https://workdrive.zohocloud.ca/file/abc','zoho'),'https://workdrive.zohocloud.ca/file/abc');
  assert.equal(safeCloudLink('https://www.zohoapis.ca/workdrive/file/abc','zoho'),'https://www.zohoapis.ca/workdrive/file/abc');
  assert.equal(safeCloudLink('https://docs.google.com/document/d/123/edit#heading=h.section','google'),'https://docs.google.com/document/d/123/edit#heading=h.section');
  for(const url of ['http://drive.google.com/file','https://drive.google.com.evil.invalid/file','https://drive.google.com@evil.invalid/file','https://user:secret@drive.google.com/file','https://drive.google.com:444/file','javascript:alert(1)','https://drive.google.com/\nfile'])assert.equal(safeCloudLink(url,'google'),null,url);
  for(const url of ['https://zohocloud.ca.evil.invalid/file','https://zoho.ca/file','https://workdrive.zohopublic.com/file'])assert.equal(safeCloudLink(url,'zoho'),null,url);
  assert.equal(safeCloudLink('https://drive.google.com/file','dropbox'),null);
  assert.equal(safeCloudLink('https://drive.google.com/file','unknown'),null);
});
test('base64 sizes count actual bytes and reject malformed data without trusting metadata',()=>{
  assert.equal(attachmentByteLength(file('zero','')),0);
  assert.equal(attachmentByteLength(file('one','a')),1);
  assert.equal(attachmentByteLength(file('two','ab')),2);
  assert.equal(attachmentByteLength(file('three','abc')),3);
  assert.equal(attachmentByteLength({...file('fake','abcdef'),size:0}),6);
  for(const data of ['a','AA=A','AAAA=','====','A===','AA\nA','@@@='])assert.throws(()=>attachmentByteLength({data}),/base64/);
});
test('local and cloud files share the aggregate account limit without dropping existing files',()=>{
  assert.equal(attachmentLimit('outlook'),3_000_000);assert.equal(attachmentLimit('gmail'),9_000_000);
  const existing=[file('cloud.txt','cloud')],next=appendAttachment(existing,file('local.txt','local'),'gmail');
  assert.deepEqual(next.map(f=>f.name),['cloud.txt','local.txt']);assert.equal(existing.length,1);
  const full={name:'full.bin',data:'AAAA'.repeat(1_000_000),source:'Cloud'};
  assert.equal(appendAttachment([],full,'outlook')[0].size,3_000_000);
  assert.throws(()=>appendAttachment([full],file('extra.txt','x'),'outlook'),/3 MB total/);
  assert.equal(existing[0].name,'cloud.txt');
  assert.throws(()=>appendAttachment([],file('bad\r\nname','x'),'gmail'),/control characters/);
});
test('the Gmail boundary is enforced for large base64 without recursive-regex failure',()=>{
  const full={name:'nine.bin',data:'AAAA'.repeat(3_000_000)};
  assert.equal(appendAttachment([],full,'gmail')[0].size,9_000_000);
  assert.throws(()=>appendAttachment([full],file('extra','x'),'gmail'),/9 MB total/);
});
test('cloud link text remains plain text and size labels handle missing or empty files',()=>{
  assert.equal(cloudLinkText({name:'Report\r\n2026'},'https://docs.google.com/document/d/x'),'Report 2026: https://docs.google.com/document/d/x');
  assert.equal(formatCloudSize(null),'Size unavailable');assert.equal(formatCloudSize(0),'0 B');assert.equal(formatCloudSize(1024),'1.0 KB');assert.equal(formatCloudSize(3_000_000),'3.0 MB');
});
test('a stale connection refresh cannot resurrect a disconnected drive or replace a newer result with an old error',async()=>{
  const originalDocument=globalThis.document,requests=[];
  globalThis.document={addEventListener(){}};
  try{
    const drives=createCloudDrives({native:true,invoke:()=>new Promise((resolve,reject)=>requests.push({resolve,reject})),$:()=>({addEventListener(){}}),esc:value=>String(value),button:()=>'',toast(){},active:()=>({id:'mail'}),canAttach:()=>true,getFiles:()=>[],setFiles(){},renderAttachments(){}});
    const old=drives.refresh(),fresh=drives.refresh();
    requests[1].resolve([{id:'fresh',provider:'google',label:'Current drive'}]);await fresh;
    requests[0].resolve([{id:'old',provider:'google',label:'Disconnected drive'}]);await old;
    assert.match(drives.settingsMarkup(),/Current drive/);assert.doesNotMatch(drives.settingsMarkup(),/Disconnected drive/);
    const staleError=drives.refresh(),latest=drives.refresh();
    requests[3].resolve([{id:'fresh',provider:'google',label:'Current drive'}]);await latest;
    requests[2].reject(Error('Old stale error'));await staleError;
    assert.match(drives.settingsMarkup(),/Current drive/);assert.doesNotMatch(drives.settingsMarkup(),/Old stale error/);
  }finally{if(originalDocument===undefined)delete globalThis.document;else globalThis.document=originalDocument}
});
