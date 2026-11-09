import test from 'node:test';
import assert from 'node:assert/strict';
import {oauthClientIdError,normalizeSignInProviders,providerSignInReady,accountSignInMarkup,createAccountSignIn} from '../ui/oauth-setup.js';

test('Microsoft setup rejects login addresses and wrong identifiers before OAuth',()=>{
  for(const provider of ['outlook','onedrive']){
    for(const value of ['', 'person@outlook.com', 'Yas Mail', 'https://entra.microsoft.com', '123456781234123412341234567890ab', '{12345678-1234-1234-1234-1234567890ab}', '12345678-1234-1234-1234-1234567890az'])assert.ok(oauthClientIdError(provider,value),value);
    for(const value of ['12345678-1234-1234-1234-1234567890ab','  ABCDEF12-3456-7890-ABCD-EF1234567890  '])assert.equal(oauthClientIdError(provider,value),'');
    assert.match(oauthClientIdError(provider,'person@outlook.com'),/Application \(client\) ID.*email address/s);
  }
});
test('Google setup accepts desktop client IDs and explains email-address mistakes',()=>{
  for(const provider of ['gmail','google']){
    assert.equal(oauthClientIdError(provider,' 123456-example.apps.googleusercontent.com '),'');
    for(const value of ['person@gmail.com','.apps.googleusercontent.com','bad.id.apps.googleusercontent.com','https://123-example.apps.googleusercontent.com','123.apps.googleusercontent.com.evil.invalid'])assert.ok(oauthClientIdError(provider,value),value);
  }
  assert.equal(oauthClientIdError('dropbox','public-app-key'),'');
  assert.ok(oauthClientIdError('zoho','person@example.com'));
  assert.ok(oauthClientIdError('dropbox','x'.repeat(4097)));
});
test('publisher status accepts only known providers and explicit configured flags',()=>{
  const value=[{provider:'gmail',configured:'true',name:'Injected name'},{provider:'outlook',configured:true},{provider:'unknown',configured:true}];
  assert.deepEqual(normalizeSignInProviders(value).map(info=>[info.provider,info.configured]),[['gmail',false],['outlook',true]]);
  assert.equal(providerSignInReady(value,'onedrive'),true);assert.equal(providerSignInReady(value,'google'),false);assert.equal(providerSignInReady(null,'gmail'),false);
});
test('consumer sign-in needs no application fields and makes build availability explicit',()=>{
  const enabled=accountSignInMarkup([{provider:'gmail',configured:true},{provider:'outlook',configured:true}]);
  assert.match(enabled,/Continue with Google/);assert.match(enabled,/Continue with Microsoft/);assert.doesNotMatch(enabled,/<input|client.?id|client.?secret|app registrations/i);assert.doesNotMatch(enabled,/ disabled/);
  const missing=accountSignInMarkup([]);assert.equal((missing.match(/ disabled/g)||[]).length,2);assert.match(missing,/unavailable|isn’t enabled/i);
  const escaped=accountSignInMarkup([],{error:'<script>provider error</script>'});assert.doesNotMatch(escaped,/<script>/);assert.match(escaped,/&lt;script&gt;/);
});

function signInHarness(){
  const calls=[],requests=[],events={};let host;
  const dialog={open:false,addEventListener:(name,listener)=>events[name]=listener,querySelectorAll:()=>[{disabled:false,isConnected:true}]};
  const $=selector=>selector==='#modal'?dialog:host;
  const connected=[];
  const controller=createAccountSignIn({native:true,invoke:(command,args)=>{calls.push({command,args});return new Promise((resolve,reject)=>requests.push({resolve,reject}))},$,modal:()=>{if(host)host.isConnected=false;const controls=new Map();host={innerHTML:'',isConnected:true,querySelector:selector=>{if(!controls.has(selector))controls.set(selector,{disabled:false,textContent:''});return controls.get(selector)}};dialog.open=true},onConnected:async account=>{connected.push(account);dialog.open=false}});
  return {controller,calls,requests,events,dialog,connected,get host(){return host}};
}
test('sign-in sends only the provider and prevents duplicate authorization while pending',async()=>{
  const h=signInHarness(),opening=h.controller.open();h.requests[0].resolve([{provider:'outlook',configured:true}]);await opening;
  const first=h.controller.connect('outlook');await h.controller.connect('outlook');await h.controller.connect('gmail');
  assert.equal(h.calls.filter(call=>call.command==='connect').length,1);assert.deepEqual(h.calls[1],{command:'connect',args:{provider:'outlook'}});assert.equal(h.controller.isPending(),true);assert.match(h.host.innerHTML,/Finish signing in in your browser/);
  let prevented=false;h.events.cancel({preventDefault(){prevented=true}});assert.equal(prevented,true);
  h.requests[1].resolve({id:'synthetic-account'});await first;assert.deepEqual(h.connected,[{id:'synthetic-account'}]);assert.equal(h.controller.isPending(),false);
});
test('missing configuration and stale availability cannot trigger or replace a newer sign-in',async()=>{
  const h=signInHarness(),old=h.controller.open(),oldHost=h.host,newer=h.controller.open();
  h.requests[1].resolve([]);await newer;h.requests[0].resolve([{provider:'gmail',configured:true}]);await old;
  assert.notEqual(h.host,oldHost);assert.match(h.host.innerHTML,/Unavailable in this preview build/);await h.controller.connect('gmail');assert.equal(h.calls.filter(call=>call.command==='connect').length,0);
});
test('provider errors restore sign-in choices without pretending authorization succeeded',async()=>{
  const h=signInHarness(),opening=h.controller.open();h.requests[0].resolve([{provider:'gmail',configured:true}]);await opening;
  const connecting=h.controller.connect('gmail');h.requests[1].reject(Error('Synthetic provider declined request'));await connecting;
  assert.equal(h.controller.isPending(),false);assert.equal(h.connected.length,0);assert.match(h.host.innerHTML,/Synthetic provider declined request/);assert.match(h.host.innerHTML,/Continue with Google/);
});
test('cancel waits for native acknowledgement and the original sign-in to settle',async()=>{
  const h=signInHarness(),opening=h.controller.open();h.requests[0].resolve([{provider:'gmail',configured:true}]);await opening;
  const connecting=h.controller.connect('gmail'),cancelling=h.controller.cancel();await h.controller.cancel();assert.equal(h.calls.filter(call=>call.command==='cancel_sign_in').length,1);
  h.requests[2].resolve(true);await cancelling;assert.equal(h.controller.isPending(),true);assert.equal(h.connected.length,0);
  h.requests[1].reject(Error('Sign-in cancelled.'));await connecting;assert.equal(h.controller.isPending(),false);assert.match(h.host.innerHTML,/Sign-in cancelled\./);assert.equal(h.connected.length,0);
  const retry=h.controller.connect('gmail');h.requests[3].reject(Error('New provider error'));await retry;assert.match(h.host.innerHTML,/New provider error/);
});
test('cancel after the callback stays pending until the native connection finishes',async()=>{
  const h=signInHarness(),opening=h.controller.open();h.requests[0].resolve([{provider:'outlook',configured:true}]);await opening;
  const connecting=h.controller.connect('outlook'),cancelling=h.controller.cancel();h.requests[2].resolve(false);await cancelling;
  assert.equal(h.controller.isPending(),true);assert.equal(h.host.querySelector('.sign-in-waiting').textContent,'Finishing sign-in…');h.requests[1].resolve({id:'completed'});await connecting;assert.deepEqual(h.connected,[{id:'completed'}]);
});
