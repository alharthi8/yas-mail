// Window identity comes from Rust's caller-bound context, never a URL parameter.
export const defaultWindowContext={label:'main',isDetached:false,totalAccounts:0,initialAccount:null,view:null};
export function normalizeWindowView(view){
  if(!view||typeof view!=='object')return {};
  const out={};
  if(['mail','calendar','contacts','settings','rules','local-drafts','outbox','attachments','snoozed','reminders'].includes(view.page))out.page=view.page;
  if(['inbox','sent','starred','drafts','archive','local-archive','spam','trash'].includes(view.folder)||(typeof view.folder==='string'&&view.folder.startsWith('custom:')&&view.folder.length<=4096&&!/[\x00-\x1f]/.test(view.folder)))out.folder=view.folder;
  if(typeof view.query==='string')out.query=view.query.slice(0,1000);
  if(typeof view.selected==='string')out.selected=view.selected.slice(0,2000);
  if(view.selected===null)out.selected=null;
  const month=new Date(view.month);if(view.month&&!Number.isNaN(month.getTime()))out.month=month;
  if(typeof view.contactFilter==='string')out.contactFilter=view.contactFilter;
  return out;
}
export function mergeVisibleTabOrder(saved,visible){
  const ids=[...new Set(visible)],visibleSet=new Set(ids),old=Array.isArray(saved)?[...new Set(saved.filter(id=>typeof id==='string'))]:[];
  let next=0;const result=old.map(id=>visibleSet.has(id)?ids[next++]:id);
  while(next<ids.length)result.push(ids[next++]);return result;
}
export function detachedView(state,accountId,views,prefs){
  const view=state.account===accountId?state:views.get(accountId)||{};
  return {page:view.page||'mail',folder:view.folder||'inbox',query:view.query||'',selected:view.selected||null,month:view.month instanceof Date?view.month.toISOString():null,contactFilter:view.contactFilter||'all',appearance:Object.fromEntries(['color','avatar'].filter(key=>prefs[accountId]?.[key]!==undefined).map(key=>[key,prefs[accountId][key]]))};
}
