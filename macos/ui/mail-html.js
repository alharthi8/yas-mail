import DOMPurify from './vendor/purify.es.mjs';

export const escapeHtml=value=>String(value??'').replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
export function safeExternalUrl(value){
  if(typeof value!=='string'||value.length>8192||/[\u0000-\u0020\u007f]/.test(value))return null;
  try{const url=new URL(value);if(url.username||url.password)return null;if(url.protocol==='https:'&&url.hostname)return url.href;if(url.protocol==='mailto:'&&url.pathname&&!/[\r\n]/i.test(decodeURIComponent(url.href)))return url.href}catch{}
  return null;
}
export function safeInlineImage(value){return typeof value==='string'&&value.length<=7_000_000&&/^data:image\/(?:png|jpeg|gif|webp);base64,[A-Za-z0-9+/=]+$/i.test(value)?value:null}
export function htmlToText(value){
  if(typeof document==='undefined')return String(value||'').replace(/<[^>]*>/g,' ').replace(/\s+/g,' ').trim();
  const node=document.createElement('div');node.innerHTML=DOMPurify.sanitize(String(value||''),{USE_PROFILES:{html:true},FORBID_TAGS:['style','script','iframe','form','svg','math']});
  node.querySelectorAll('br').forEach(br=>br.replaceWith('\n'));node.querySelectorAll('p,div,li,tr').forEach(el=>el.append('\n'));
  return (node.textContent||'').replace(/\n{3,}/g,'\n\n').trim();
}
export function sanitizeRichHtml(value){
  const clean=DOMPurify.sanitize(String(value||'').slice(0,5_000_000),{USE_PROFILES:{html:true},SANITIZE_NAMED_PROPS:true,ALLOW_DATA_ATTR:false,FORBID_TAGS:['style','script','iframe','frame','frameset','form','input','button','textarea','select','object','embed','svg','math','video','audio','source','link','meta','base'],FORBID_ATTR:['srcset','background','ping','target','download','id','name']});
  if(typeof document==='undefined')return clean;
  const box=document.createElement('div');box.innerHTML=clean;
  box.querySelectorAll('*').forEach(node=>{
    const style=node.getAttribute('style');if(style&&/(?:url\s*\(|@import|expression\s*\(|-moz-binding|behavior\s*:)/i.test(style))node.removeAttribute('style');
    if(node.tagName==='IMG'){const src=node.getAttribute('src')||'';if(!safeInlineImage(src)&&!/^cid:[A-Za-z0-9_.@-]+$/.test(src)){node.replaceWith(document.createTextNode(node.getAttribute('alt')||'[image]'));return}node.removeAttribute('loading');node.setAttribute('referrerpolicy','no-referrer')}
    if(node.tagName==='A'){const href=safeExternalUrl(node.getAttribute('href'));if(href)node.setAttribute('href',href);else node.removeAttribute('href')}
  });
  return box.innerHTML;
}
export function emailDocument(value,{remoteImages=false,dark=false,inlineImages={},remoteImageData={}}={}){
  const clean=DOMPurify.sanitize(String(value||'').slice(0,5_000_000),{USE_PROFILES:{html:true},SANITIZE_NAMED_PROPS:true,ALLOW_DATA_ATTR:false,FORBID_TAGS:['script','iframe','frame','frameset','form','input','button','textarea','select','object','embed','svg','math','video','audio','source','link','meta','base','style'],FORBID_ATTR:['srcset','background','ping','target','download','id','name']});
  const box=document.createElement('div');box.innerHTML=clean;const links=[],remoteImageUrls=[];let blockedImages=0;
  box.querySelectorAll('*').forEach(node=>{
    const style=node.getAttribute('style');if(style&&/(?:url\s*\(|@import|expression\s*\(|-moz-binding|behavior\s*:)/i.test(style))node.removeAttribute('style');
    if(node.tagName==='A'){
      const href=safeExternalUrl(node.getAttribute('href'));node.removeAttribute('href');
      if(href){let index=links.findIndex(link=>link.url===href);if(index<0){index=links.length;links.push({url:href,label:(node.textContent||href).trim().slice(0,140)})}node.append(document.createTextNode(' ['+(index+1)+']'))}
    }
    if(node.tagName==='IMG'){
      let src=node.getAttribute('src')||'';if(src.startsWith('cid:'))src=inlineImages[src.slice(4)]||'';
      const embedded=safeInlineImage(src)||safeInlineImage(remoteImageData[src]),remote=safeExternalUrl(src);
      if(remote?.startsWith('https:')&&!remoteImageUrls.includes(remote))remoteImageUrls.push(remote);
      if(embedded)node.setAttribute('src',embedded);else{blockedImages++;const placeholder=document.createElement('span');placeholder.className='blocked-image';placeholder.textContent=node.getAttribute('alt')||'[Image blocked]';node.replaceWith(placeholder);return}
      node.setAttribute('referrerpolicy','no-referrer');node.setAttribute('loading','lazy');node.removeAttribute('srcset');
    }
  });
  const csp="default-src 'none'; script-src 'none'; connect-src 'none'; object-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; media-src 'none'; font-src 'none'; style-src 'unsafe-inline'; img-src data:;";
  return {html:'<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="'+escapeHtml(csp)+'"><meta name="referrer" content="no-referrer"><style>html{color-scheme:'+(dark?'dark':'light')+'}body{font:14px/1.65 Segoe UI,sans-serif;margin:16px;overflow-wrap:anywhere;background:'+(dark?'#101d33':'#fff')+';color:'+(dark?'#e2eaf7':'#253c5e')+'}img{max-width:100%;height:auto}table{max-width:100%;border-collapse:collapse}td,th{padding:4px}pre{white-space:pre-wrap}.blocked-image{display:inline-block;padding:8px;border:1px dashed #8ca1be;color:#6d84a3;border-radius:5px}a{color:#2563eb}blockquote{border-left:3px solid #bacbe4;margin-left:0;padding-left:14px}</style></head><body>'+box.innerHTML+'</body></html>',links,blockedImages,remoteImageUrls:remoteImageUrls.slice(0,10)};
}
export function conversationKey(message){return String(message.threadId||message.conversationId||message.internetMessageId||message.id||'')}
export function groupConversations(messages){
  const groups=new Map();for(const message of messages){const key=String(message.accountId||'')+'\u0000'+conversationKey(message);if(!groups.has(key))groups.set(key,[]);groups.get(key).push(message)}
  return [...groups.values()].map(items=>({key:String(items[0].accountId||'')+'\u0000'+conversationKey(items[0]),items:items.sort((a,b)=>new Date(a.date)-new Date(b.date)),latest:items.reduce((a,b)=>new Date(a.date)>new Date(b.date)?a:b),unread:items.some(item=>item.unread)})).sort((a,b)=>new Date(b.latest.date)-new Date(a.latest.date));
}
