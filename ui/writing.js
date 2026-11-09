export function checkWriting(text){
 const issues=[];
 if(/\b(\w+)\s+\1\b/i.test(text))issues.push('Repeated word: review adjacent duplicate words.');
 if(/\bi\b/.test(text))issues.push('Capitalize “I” when referring to yourself.');
 if(/\b(your welcome|could of|should of|would of)\b/i.test(text))issues.push('Check “you’re welcome” or “could/should/would have”.');
 if(/\b(teh|recieve|seperate|definately|occured)\b/i.test(text))issues.push('Possible spelling error: check commonly misspelled words.');
 if(/[a-z][.!?][A-Z]/.test(text))issues.push('Add a space after sentence punctuation.');
 if(/\w[ \t]+[,.;!?]/.test(text))issues.push('Remove extra spaces before punctuation.');
 if(text.split(/[.!?]+/).some(s=>s.trim().split(/\s+/).length>45))issues.push('A sentence is quite long. Consider splitting it for clarity.');
 return issues;
}
export const templates=[['Thanks','Thank you for the update. I appreciate you sharing this.'],['Sounds good','That sounds good to me. Please go ahead, and let me know if you need anything from me.'],['Need a moment','Thanks for your message. I’ll review the details and get back to you shortly.'],['Schedule a meeting','Thanks for reaching out. Could you share a few times that work for a quick meeting?'],['Follow up','I wanted to follow up on my previous message. Please let me know when you have a chance to review it.']];
export function createWritingTools({$,esc,button,pref,invoke,toast,active}){
 return async function writingAction(a,el){
  if(a==='ai-check-runtime'){const status=$('#ai-status');status.textContent='Checking local runtime…';try{const v=await invoke('ai_models');status.textContent=v.length?'Local models available: '+v.join(', '):'Ollama is running. No local models installed. Run: ollama pull gemma3:1b';}catch(e){status.textContent=String(e)}return}
  if(a==='ai-clear-memory'){try{await invoke('style_clear',{account:active().id});toast('Local writing-style memory cleared')}catch(e){toast(String(e))}return}
  const panel=$('#writing-panel');if(!panel)return;
  if(a==='writing-check'){const issues=checkWriting($('#body').value);panel.innerHTML=`<div class="notice"><strong>Local writing check</strong><p>${issues.length?issues.map(esc).join('<br>'):'No common issues found by the basic checker.'}</p><span class="muted">This is a limited rule-based check. Windows/WebView spelling suggestions depend on installed languages. Use AI rewrite for a broader grammar review.</span></div>`;return}
  if(a==='reply-templates'){panel.innerHTML=`<div class="notice"><strong>Quick replies · templates</strong><div class="template-list">${templates.map(([title],i)=>button(esc(title),'use-template','quiet',`data-index="${i}"`)).join('')}</div></div>`;return}
  if(a==='use-template'){const input=$('#body');input.setRangeText(templates[Number(el.dataset.index)][1],input.selectionStart,input.selectionEnd,'end');input.focus();return}
  if(a==='ai-rewrite'){panel.innerHTML=`<div class="ai-panel"><div class="section-title"><strong>✦ Yas Mail writing assistant</strong><span class="pill">LOCAL AI</span></div><div class="two"><label class="field">Email tone<select id="ai-tone" aria-label="Email tone">${['Professional','Friendly','Warm','Concise','Confident','Neutral'].map(t=>`<option>${t}</option>`).join('')}</select></label><label class="field">What would you like to do?<select id="ai-task"><option value="rewrite">Rewrite and improve clarity</option><option value="shorten">Make it shorter</option><option value="grammar">Fix grammar and spelling</option></select></label></div><p class="muted">Only this draft and, if enabled, local style examples are sent to Ollama on this computer. Model: ${esc(pref().aiModel||'gemma3:1b')}. Review before applying.</p>${button('Generate preview','ai-generate','primary')}<div id="ai-result" aria-live="polite"></div></div>`;return}
  if(a==='ai-generate'){const result=$('#ai-result');const original=$('#body').value;if(!original.trim()){result.textContent='Write a draft first.';return}const tone=$('#ai-tone').value,task=$('#ai-task').value;el.disabled=true;result.textContent='Rewriting locally…';try{const output=await invoke('ai_rewrite',{account:active().id,body:original,tone,task,model:pref().aiModel||'gemma3:1b',useStyle:!!pref().learnStyle});if(!$('#ai-result'))return;panel._original=original;panel._suggestion=output;result.innerHTML=`<label class="field" style="margin-top:18px">Suggested version<textarea id="ai-preview" spellcheck="true">${esc(output)}</textarea></label><div class="modal-footer">${button('Copy suggestion','ai-copy')}${button('Use this version','ai-apply','primary')}</div>`;}catch(e){result.textContent=String(e)}finally{el.disabled=false}return}
  if(a==='ai-apply'){if($('#body').value!==panel._original){toast('Your draft changed while AI was working. Copy the suggestion to avoid losing your changes.');return}$('#body').value=$('#ai-preview').value;panel.innerHTML='';toast('Suggested version applied. Review before sending.');return}
  if(a==='ai-copy'){try{await navigator.clipboard.writeText($('#ai-preview').value);toast('Suggestion copied')}catch{toast('Select and copy the suggestion manually.')}return}
 }
}
