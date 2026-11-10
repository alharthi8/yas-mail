import test from 'node:test';
import assert from 'node:assert/strict';
import {parseCsv,parseContacts,exportContacts,contactKey} from '../ui/contacts-io.js';
import {scanContacts,suggestName} from '../ui/contact-health.js';
import {checkWriting} from '../ui/writing.js';
test('CSV handles quotes, commas, multiline fields, CRLF and BOM',()=>{
 assert.deepEqual(parseCsv('\uFEFFName,Email,Phone\r\n"Chen, Maya",maya@example.com,"+1 202\n555"\r\n'),[['Name','Email','Phone'],['Chen, Maya','maya@example.com','+1 202\n555']]);
 assert.throws(()=>parseCsv('Name\n"unterminated'),/unclosed/);
});
test('CSV imports Google/Outlook-style column names',()=>{
 const contacts=parseContacts('First Name,Last Name,E-mail Address,Mobile Phone\nMaya,Chen,maya@example.com,+12025550101','contacts.csv');
 assert.deepEqual(contacts,[{name:'Maya Chen',email:'maya@example.com',phone:'+12025550101'}]);
});
test('vCard export and import preserve UTF-8 and escaped punctuation',()=>{
 const contacts=[{name:'Maya, 陈; Team',email:'maya@example.com',phone:'+12025550101'}];
 assert.deepEqual(parseContacts(exportContacts(contacts,'vcf'),'contacts.vcf'),contacts);
});
test('import rejects malformed email and excessive batches',()=>{
 assert.throws(()=>parseContacts('Name,Email\nTest,not-an-email','x.csv'),/Invalid email/);
 assert.throws(()=>parseContacts('Name,Email\n'+Array.from({length:501},(_,i)=>`Person ${i},person${i}@example.com`).join('\n'),'x.csv'),/500/);
});
test('CSV export prevents spreadsheet formula execution',()=>{
 const csv=exportContacts([{name:'=HYPERLINK("bad")',email:'a@example.com',phone:'+12025550101'}],'csv');
 assert.ok(csv.includes('"\'=HYPERLINK'));assert.ok(csv.includes('"\'+12025550101"'));
});
test('deduplication uses email case and explicit primary phone matches',()=>{
 const report=scanContacts([{id:'1',name:'',email:'MAYA@example.com',phone:''},{id:'2',name:'Maya',email:'maya@example.com',phone:'+1 (202) 555-0101'},{id:'3',name:'Different person',email:'other@example.com',phone:'+12025550101'}]);
 assert.equal(report.counts['duplicate'],3);assert.equal(report.counts['missing-name'],1);assert.equal(report.counts['missing-phone'],1);
 assert.equal(contactKey({email:'MAYA@example.com'}),contactKey({email:'maya@example.com'}));
});
test('name suggestions are derived, never invented',()=>{assert.equal(suggestName('maya.chen@example.com'),'Maya Chen');assert.equal(suggestName('12345@example.com'),'')});
test('writing checker catches common issues and accepts ordinary prose',()=>{
 assert.ok(checkWriting('i could of sent teh the the file.').length>=3);
 assert.deepEqual(checkWriting('Thank you for your message. I will review it tomorrow.'),[]);
});
