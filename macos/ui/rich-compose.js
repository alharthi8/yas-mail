import {escapeHtml,sanitizeRichHtml,htmlToText,safeExternalUrl,safeInlineImage} from './mail-html.js';

export function signaturePresets(prefs={}){
  const presets=Array.isArray(prefs.signatures)?prefs.signatures.filter(item=>item&&typeof item.id==='string'&&typeof item.html==='string').slice(0,20):[];
  return presets.length?presets:prefs.signature?[{id:'default',name:'Default',html:'<p>'+escapeHtml(prefs.signature).replace(/\n/g,'<br>')+'</p>'}]:[];
}
export function preferredSignature(prefs={},mode='new'){
  const items=signaturePresets(prefs),id=mode==='new'?prefs.newSignature:prefs.replySignature;
  if(id==='none')return null;return items.find(item=>item.id===id)||items[0]||null;
}
export function richEditorMarkup(html=''){
  return '<div class="rich-compose"><div class="rich-toolbar" role="toolbar" aria-label="Message formatting">'+[
    ['bold','Bold','<strong>B</strong>'],['italic','Italic','<i>I</i>'],['underline','Underline','<u>U</u>'],['insertUnorderedList','Bullet list','• List'],['insertOrderedList','Numbered list','1. List'],['formatBlock','Heading','Heading'],['removeFormat','Clear formatting','Clear']
  ].map(([cmd,label,text])=>'<button type="button" data-rich-command="'+cmd+'" aria-label="'+label+'" title="'+label+'">'+text+'</button>').join('')+'<details class="rich-details"><summary>Link</summary><input type="url" data-rich-link aria-label="Secure link URL" placeholder="https://…"><button type="button" data-action="rich-link">Insert link</button></details><button type="button" data-action="rich-table">Table</button><label class="rich-image-label">Inline image<input data-rich-image type="file" accept="image/png,image/jpeg,image/gif,image/webp" hidden></label></div><div id="rich-body" class="rich-body" contenteditable="true" role="textbox" aria-label="Rich message" aria-multiline="true" spellcheck="true">'+sanitizeRichHtml(html)+'</div><p class="muted rich-help">Formatting stays in the email. Your local AI and writing checker use the plain-text version.</p></div>';
}
export function createRichEditor({host,textarea,getFiles,setFiles,onChange=()=>{},onError=()=>{}}){
  const editor=host.querySelector('#rich-body');let changing=false,lastRange=null,active=true;
  const imageSources=new Map();
  for(const file of getFiles()){if(file.inline&&file.cid&&file.contentType)imageSources.set(file.cid,'data:'+file.contentType+';base64,'+file.data)}
  for(const image of editor.querySelectorAll('img')){const src=image.getAttribute('src')||'';if(src.startsWith('cid:')&&imageSources.has(src.slice(4)))image.src=imageSources.get(src.slice(4))}
  function remember(){const selection=window.getSelection();if(selection?.rangeCount&&editor.contains(selection.anchorNode))lastRange=selection.getRangeAt(0).cloneRange()}
  function focus(){editor.focus();if(lastRange){const selection=window.getSelection();selection.removeAllRanges();selection.addRange(lastRange)}}
  function plainChanged(){if(changing||!active)return;changing=true;editor.innerHTML='<p>'+escapeHtml(textarea.value).replace(/\n/g,'<br>')+'</p>';changing=false;onChange()}
  function htmlChanged(){if(changing||!active)return;changing=true;textarea.value=editor.innerText||htmlToText(editor.innerHTML);textarea.dispatchEvent(new Event('input',{bubbles:true}));changing=false;remember();onChange()}
  textarea.addEventListener('input',plainChanged);editor.addEventListener('input',htmlChanged);editor.addEventListener('keyup',remember);editor.addEventListener('mouseup',remember);
  editor.addEventListener('paste',event=>{event.preventDefault();const html=event.clipboardData?.getData('text/html'),text=event.clipboardData?.getData('text/plain')||'';document.execCommand('insertHTML',false,html?sanitizeRichHtml(html):escapeHtml(text).replace(/\n/g,'<br>'));htmlChanged()});
  host.querySelectorAll('[data-rich-command]').forEach(button=>{button.addEventListener('mousedown',event=>event.preventDefault());button.addEventListener('click',()=>{focus();const command=button.dataset.richCommand;document.execCommand(command,false,command==='formatBlock'?'h3':null);htmlChanged()})});
  host.querySelector('[data-rich-image]')?.addEventListener('change',async event=>{
    const file=event.target.files?.[0];event.target.value='';if(!file)return;
    if(!/^image\/(png|jpeg|gif|webp)$/.test(file.type)||file.size>2_000_000)return onError('Choose a PNG, JPEG, GIF, or WebP image up to 2 MB.');
    try{const src=await new Promise((resolve,reject)=>{const reader=new FileReader();reader.onload=()=>resolve(String(reader.result));reader.onerror=()=>reject(Error('Image could not be read.'));reader.readAsDataURL(file)});if(!active||!editor.isConnected)return;focus();document.execCommand('insertHTML',false,'<img alt="'+escapeHtml(file.name)+'" src="'+src+'">');htmlChanged()}catch(error){onError(String(error))}
  });
  htmlChanged();
  return {
    html(){const copy=document.createElement('div');copy.innerHTML=sanitizeRichHtml(editor.innerHTML);const current=[...getFiles()];for(const image of copy.querySelectorAll('img')){const src=image.getAttribute('src');if(!safeInlineImage(src))continue;let file=current.find(item=>item.inline&&item.data===src.split(',')[1]);if(!file){const cid='yas-'+crypto.randomUUID()+'@inline';const contentType=src.slice(5,src.indexOf(';'));file={name:'inline-image.'+contentType.split('/')[1],data:src.split(',')[1],inline:true,cid,contentType,size:Math.floor(src.split(',')[1].length*3/4)};current.push(file)}image.setAttribute('src','cid:'+file.cid)}setFiles(current);return copy.innerHTML},
    setHtml(html){editor.innerHTML=sanitizeRichHtml(html);htmlChanged()},
    appendHtml(html){editor.insertAdjacentHTML('beforeend',sanitizeRichHtml(html));htmlChanged()},
    plainChanged,
    handle(action){if(action==='rich-link'){const input=host.querySelector('[data-rich-link]'),url=safeExternalUrl(input.value.trim());if(!url)return onError('Enter an HTTPS or mailto link.');focus();document.execCommand('createLink',false,url);input.value='';input.closest('details').open=false;htmlChanged();return true}if(action==='rich-table'){focus();document.execCommand('insertHTML',false,'<table border="1" cellpadding="6"><tbody><tr><th>Heading</th><th>Heading</th></tr><tr><td>Text</td><td>Text</td></tr></tbody></table><p><br></p>');htmlChanged();return true}return false},
    dispose(){active=false;textarea.removeEventListener('input',plainChanged)}
  };
}
