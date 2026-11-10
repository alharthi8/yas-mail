import test from 'node:test';
import assert from 'node:assert/strict';
import {accountAvatar,accountColor,ensureAccountAppearance,renderAccountTabs,nextAccountTab} from '../ui/account-tabs.js';
const accounts=[{id:'gmail:one',email:'one@example.com',provider:'gmail'},{id:'outlook:two',email:'two@example.com',provider:'outlook'}];
test('account avatar fallback is shared across tabs and app surfaces',()=>{
  assert.equal(accountAvatar(accounts[0],{}),'ON');
  assert.equal(accountAvatar({id:'demo',email:'alex@example.com'},{}),'YM');
  assert.equal(accountAvatar(accounts[0],{[accounts[0].id]:{avatar:'📬'}}),'📬');
  assert.equal(accountAvatar({id:'maya',email:'maya.chen@example.com'},{}),'MC');
});
test('account tabs use saved avatars and colors and expose a single selected tab',()=>{
  const html=renderAccountTabs(accounts,'outlook:two',{'outlook:two':{avatar:'📬',color:'#3e5e9c'}});
  assert.ok(html.includes('📬'));assert.ok(html.includes('--account-color:#3e5e9c'));assert.ok(html.includes('Microsoft'));assert.ok(html.includes('Google'));
  assert.equal((html.match(/aria-selected="true"/g)||[]).length,1);assert.equal((html.match(/tabindex="0"/g)||[]).length,1);
  assert.equal((html.match(/aria-controls="account-workspace"/g)||[]).length,2);assert.ok(!html.includes('Close account'));
});
test('account tab text and attributes escape untrusted account strings',()=>{
  const html=renderAccountTabs([{id:'x" data-action="bad',email:'<img src=x onerror=bad>@example.com',provider:'gmail'}],'x" data-action="bad',{'x" data-action="bad':{avatar:'<script>&"'}});
  assert.ok(!html.includes('<img'));assert.ok(!html.includes('<script>'));assert.ok(html.includes('&lt;img'));assert.ok(html.includes('x&quot; data-action=&quot;bad'));
});
test('invalid or unreadable custom colors fall back to safe deterministic colors',()=>{
  const account=accounts[0];
  for(const color of ['#ffffff','red','url(evil)','#123456;display:none'])assert.equal(accountColor(account,{[account.id]:{color}}),accountColor(account,{}));
  assert.equal(accountColor(account,{[account.id]:{color:'#28644F'}}),'#28644f');
});
test('default colors are distinct, saved, and stable across later account additions',()=>{
  const prefs={};assert.equal(ensureAccountAppearance(accounts,prefs),true);assert.notEqual(prefs[accounts[0].id].color,prefs[accounts[1].id].color);
  const original=JSON.stringify(prefs);assert.equal(ensureAccountAppearance(accounts,prefs),false);assert.equal(JSON.stringify(prefs),original);
  ensureAccountAppearance([...accounts,{id:'third',email:'third@example.com',provider:'gmail'}],prefs);assert.equal(JSON.stringify(Object.fromEntries(accounts.map(a=>[a.id,prefs[a.id]]))),original);
});
test('empty account list renders one honest demo tab',()=>{
  const html=renderAccountTabs([],'demo',{});assert.equal((html.match(/role="tab"/g)||[]).length,1);assert.ok(html.includes('Demo workspace'));assert.ok(html.includes('>YM<'));assert.ok(!html.includes('alex@example'));
});
test('tab keyboard navigation wraps, supports endpoints, and ignores other keys',()=>{
  assert.equal(nextAccountTab(accounts,'gmail:one','ArrowLeft'),'outlook:two');assert.equal(nextAccountTab(accounts,'outlook:two','ArrowRight'),'gmail:one');
  assert.equal(nextAccountTab(accounts,'outlook:two','Home'),'gmail:one');assert.equal(nextAccountTab(accounts,'gmail:one','End'),'outlook:two');
  assert.equal(nextAccountTab(accounts,'gmail:one','Enter'),null);assert.equal(nextAccountTab([],'demo','ArrowRight'),'demo');assert.equal(nextAccountTab(accounts,'missing','ArrowRight'),'gmail:one');
});
