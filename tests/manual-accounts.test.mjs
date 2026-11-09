import test from 'node:test';
import assert from 'node:assert/strict';
import {accountCapabilities,accountFolders,accountProviderLabel,serverDefaultPort,manualSettingsError} from '../ui/manual-accounts.js';
import {accountSignInMarkup} from '../ui/oauth-setup.js';
const settings=()=>({email:'person@example.invalid',displayName:'Example Person',protocol:'imap',incoming:{host:'mail.example.invalid',port:993,security:'tls',username:'person@example.invalid',auth:'password'},outgoing:{host:'smtp.example.invalid',port:587,security:'starttls',username:'person@example.invalid',auth:'password'}});
const credentials={incomingPassword:'synthetic-incoming-password',smtpPassword:'synthetic-smtp-password'};

test('manual server defaults distinguish protocol and required encryption',()=>{
  assert.equal(serverDefaultPort('imap','tls'),993);assert.equal(serverDefaultPort('imap','starttls'),143);
  assert.equal(serverDefaultPort('pop3','tls'),995);assert.equal(serverDefaultPort('pop3','starttls'),110);
  assert.equal(serverDefaultPort('smtp','tls'),465);assert.equal(serverDefaultPort('smtp','starttls'),587);
});
test('manual validation requires explicit mail servers and credentials without issuing requests',()=>{
  assert.equal(manualSettingsError(settings(),credentials),'');
  for(const host of ['https://mail.example.invalid','user@mail.example.invalid','mail.example.invalid/path','mail.example.invalid:993','mail.exa\nmple.invalid']){const value=settings();value.incoming.host=host;assert.ok(manualSettingsError(value,credentials),host)}
  for(const port of [0,65536,1.5,NaN]){const value=settings();value.outgoing.port=port;assert.match(manualSettingsError(value,credentials),/port/)}
  assert.match(manualSettingsError(settings(),{incomingPassword:'',smtpPassword:'x'}),/Incoming.*password/);
  assert.match(manualSettingsError(settings(),{incomingPassword:'x',smtpPassword:''}),/SMTP.*password/);
  const display=settings();display.displayName='Name\r\nBcc: other@example.invalid';assert.match(manualSettingsError(display,credentials),/display name/);
});
test('manual setup accepts local IP servers while rejecting plaintext and unsupported auth',()=>{
  const value=settings();value.incoming.host='127.0.0.1';value.outgoing.host='[::1]';assert.equal(manualSettingsError(value,credentials),'');
  value.incoming.security='none';assert.match(manualSettingsError(value,credentials),/TLS/);value.incoming.security='tls';
  value.incoming.auth='none';assert.match(manualSettingsError(value,credentials),/authentication/);value.incoming.auth='password';
  value.outgoing.auth='oauth-pretend';assert.match(manualSettingsError(value,credentials),/authentication/);
});
test('SMTP-only encrypted relays need no incoming server or invented password',()=>{
  const value=settings();value.protocol='smtp';value.incoming=null;value.outgoing.auth='none';value.outgoing.username='';
  assert.equal(manualSettingsError(value,{incomingPassword:'',smtpPassword:''}),'');
  value.outgoing.security='none';assert.match(manualSettingsError(value,{}),/TLS/);
});
test('manual capabilities keep POP3 read only and SMTP send only without provider feature fallbacks',()=>{
  const imap=accountCapabilities({provider:'imap'}),pop=accountCapabilities({provider:'pop3'}),smtp=accountCapabilities({provider:'smtp'});
  assert.equal(imap.drafts,true);assert.equal(imap.contacts,false);assert.equal(imap.calendar,false);assert.equal(imap.rules,false);
  assert.equal(pop.mailRead,true);assert.equal(pop.mailActions,false);assert.equal(pop.drafts,false);assert.equal(pop.mailSend,true);
  assert.equal(smtp.mailRead,false);assert.equal(smtp.mailSend,true);assert.equal(smtp.attachments,false);
  assert.equal(accountCapabilities({provider:'gmail'}).contacts,true);assert.equal(accountCapabilities({provider:'outlook',capabilities:{calendar:false}}).calendar,false);
  const folders=[['inbox'],['sent'],['drafts']];assert.deepEqual(accountFolders({provider:'pop3'},folders),[['inbox']]);assert.deepEqual(accountFolders({provider:'smtp'},folders),[]);assert.equal(folders.length,3);
  assert.equal(accountProviderLabel({provider:'pop3'}),'POP3 + SMTP');
});
test('manual setup remains available when publisher sign-in is unavailable',()=>{
  const html=accountSignInMarkup([]);assert.match(html,/data-action="manual-open"/);assert.match(html,/IMAP, POP3, SMTP/);assert.match(html,/Microsoft 365 &amp; Exchange|Microsoft 365 & Exchange/);
  assert.doesNotMatch(html.match(/<button[^>]*data-action="manual-open"[^>]*>/)[0],/disabled/);
});
