import test from 'node:test';
import assert from 'node:assert/strict';
import {restoreAccountOrder,moveAccount} from '../ui/tab-order.js';

const accounts=Object.freeze(['personal','work','studio','family'].map(id=>Object.freeze({id,email:`${id}@example.com`})));
const ids=items=>items.map(account=>account.id);

test('saved order ignores disconnected IDs and duplicates, then appends new accounts',()=>{
  const saved=Object.freeze(['missing','studio','personal','studio','retired']);
  const restored=restoreAccountOrder(accounts,saved);
  assert.deepEqual(ids(restored),['studio','personal','work','family']);
  assert.deepEqual(ids(accounts),['personal','work','studio','family']);
  assert.notStrictEqual(restored,accounts);
  for(const account of restored)assert.strictEqual(account,accounts.find(original=>original.id===account.id));
});

test('invalid or empty saved order preserves provider order in a fresh array',()=>{
  for(const saved of [undefined,null,'work',{},[],[null,3,{},'missing']]){
    const restored=restoreAccountOrder(accounts,saved);
    assert.deepEqual(restored,accounts);assert.notStrictEqual(restored,accounts);
  }
});

test('new accounts append after the saved connected accounts in provider order',()=>{
  const restored=restoreAccountOrder(accounts,['work','personal']);
  assert.deepEqual(ids(restored),['work','personal','studio','family']);
  assert.deepEqual(ids(restoreAccountOrder(accounts,['family','studio','work','personal'])),['family','studio','work','personal']);
});

test('moving a tab forward uses the requested final absolute position',()=>{
  assert.deepEqual(ids(moveAccount(accounts,'personal',3)),['work','studio','family','personal']);
  assert.deepEqual(ids(moveAccount(accounts,'work',2)),['personal','studio','work','family']);
});

test('moving backward and clamping at either end preserve account objects',()=>{
  const moved=moveAccount(accounts,'family',0);
  assert.deepEqual(ids(moved),['family','personal','work','studio']);
  assert.strictEqual(moved[0],accounts[3]);
  assert.deepEqual(ids(moveAccount(accounts,'studio',-100)),['studio','personal','work','family']);
  assert.deepEqual(ids(moveAccount(accounts,'personal',100)),['work','studio','family','personal']);
  assert.deepEqual(ids(moveAccount(accounts,'personal',Infinity)),['work','studio','family','personal']);
  assert.deepEqual(ids(moveAccount(accounts,'family',-Infinity)),['family','personal','work','studio']);
  assert.deepEqual(ids(accounts),['personal','work','studio','family']);
});

test('unknown IDs, unchanged positions, and invalid positions return unchanged copies',()=>{
  for(const [id,index]of [['missing',0],['work',1],['work',NaN],['work','2'],['work',undefined]]){
    const moved=moveAccount(accounts,id,index);
    assert.deepEqual(moved,accounts);assert.notStrictEqual(moved,accounts);
  }
  assert.deepEqual(ids(moveAccount(accounts,'family',1.9)),['personal','family','work','studio']);
});

test('empty account collections restore and move safely',()=>{
  assert.deepEqual(restoreAccountOrder([],['missing']),[]);
  assert.deepEqual(moveAccount([],'missing',0),[]);
});
