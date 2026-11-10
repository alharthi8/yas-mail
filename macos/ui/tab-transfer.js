export function insertionIndex(tabRects,dropX){
  if(!Number.isFinite(dropX))return tabRects.length;
  return tabRects.filter(rect=>dropX>(rect.left+rect.right)/2).length;
}
export function insertTransferredAccount(accounts,id,index){
  const account=accounts.find(account=>account.id===id);if(!account)return [...accounts];
  const rest=accounts.filter(account=>account.id!==id),at=Math.max(0,Math.min(rest.length,Number.isFinite(index)?Math.trunc(index):rest.length));
  rest.splice(at,0,account);return rest;
}
export function createTabTransfer({native,invoke,isDragging,onError=console.error}){
  let sequence=0,lastProbe=0,timer=null,lastPoint=null,enabled=false,marker=null,lastBlocked=null;
  let blockedSequence=0,blockedQueue=Promise.resolve();
  let incoming=null,receiverFrame=0,receiverTime=null;
  const receivingBlocked=()=>!!document.querySelector('dialog[open]')||isDragging();
  function clearMarker(){
    cancelAnimationFrame(receiverFrame);receiverFrame=0;receiverTime=null;incoming=null;marker?.remove();marker=null;
  }
  function positionMarker(){
    const strip=document.querySelector('.account-titlebar');if(!strip)return;
    const list=strip.querySelector('.account-tabs'),tabs=[...list?.querySelectorAll('[data-account-tab]')||[]].map(tab=>tab.getBoundingClientRect());
    const index=insertionIndex(tabs,incoming.dropX);
    const controls=document.querySelector('.account-titlebar .window-controls')?.getBoundingClientRect();
    let x=index<tabs.length?tabs[index].left-3:tabs.length?tabs.at(-1).right+2:46;
    const viewport=list?.getBoundingClientRect(),left=viewport?.width?Math.max(42,viewport.left-3):42;
    const right=viewport?.width?viewport.right-2:controls?controls.left-8:innerWidth-8;
    x=Math.max(left,Math.min(right,x));
    if(!marker){marker=document.createElement('div');marker.className='tab-transfer-marker';marker.setAttribute('aria-hidden','true');document.body.append(marker)}
    marker.style.left=`${x}px`;
  }
  function receiverScroll(now){
    receiverFrame=0;if(!incoming)return;
    if(receivingBlocked()){clearMarker();return}
    const elapsed=receiverTime===null?0:Math.min(40,Math.max(0,now-receiverTime));receiverTime=now;
    const list=document.querySelector('.account-tabs');
    if(list&&list.scrollWidth>list.clientWidth&&Number.isFinite(incoming.dropX)){
      const rect=list.getBoundingClientRect(),edge=38,x=incoming.dropX;
      const speed=x<rect.left+edge?-Math.min(18,(rect.left+edge-x)/2):x>rect.right-edge?Math.min(18,(x-rect.right+edge)/2):0;
      if(speed&&elapsed)list.scrollLeft+=speed*elapsed/(1000/60);
    }
    positionMarker();receiverFrame=requestAnimationFrame(receiverScroll);
  }
  function showPreview(payload){
    if(!payload?.active||receivingBlocked()){clearMarker();return}
    incoming={dropX:payload.dropX};positionMarker();
    if(!receiverFrame)receiverFrame=requestAnimationFrame(receiverScroll);
  }
  function clearPreview(){
    ++sequence;clearTimeout(timer);timer=null;lastPoint=null;
    const ghost=document.querySelector('.tab-drag-preview');ghost?.classList.remove('will-attach');
    if(enabled)invoke('clear_tab_drop').catch(()=>{});
  }
  async function probe(){
    timer=null;if(!enabled||!lastPoint||!isDragging())return;
    const ticket=++sequence,release={...lastPoint};lastProbe=performance.now();
    try{
      const target=await invoke('probe_tab_drop',{release});
      if(ticket!==sequence||!isDragging())return;
      const ghost=document.querySelector('.tab-drag-preview');if(!ghost)return;
      ghost.classList.toggle('will-attach',!!target?.label);
      if(target?.label)ghost.dataset.dropLabel='Release to move into '+(target.title||'Yas Mail');
    }catch{ /* Drop previews are advisory; the final native move validates again. */ }
  }
  function externalMove(point){
    if(!enabled)return;
    if(!point){if(lastPoint)clearPreview();return}
    lastPoint=point;
    if(timer)return;
    const delay=Math.max(0,100-(performance.now()-lastProbe));timer=setTimeout(probe,delay);
  }
  function updateBlocked(){
    if(!enabled)return;
    const blocked=receivingBlocked();if(blocked)clearMarker();if(blocked===lastBlocked)return;lastBlocked=blocked;
    const request=++blockedSequence;
    // Native async commands may finish on different worker threads. Preserve
    // every transition so a late drag-start cannot overwrite its drag-end.
    blockedQueue=blockedQueue.catch(()=>{}).then(()=>invoke('set_tab_drop_blocked',{blocked})).catch(error=>{
      if(request===blockedSequence)lastBlocked=null;
      onError(error);
    });
  }
  return {externalMove,clearPreview,showPreview,updateBlocked,
    async init(){
      if(!native)return;
      const label=window.__TAURI__.window.getCurrentWindow().label;
      await window.__TAURI__.event.listen('yasmail:tab-drop-preview',event=>showPreview(event.payload),{target:{kind:'Window',label}});
      enabled=true;updateBlocked();
      const modal=document.querySelector('dialog');if(modal)new MutationObserver(updateBlocked).observe(modal,{attributes:true,attributeFilter:['open']});
      window.addEventListener('pagehide',()=>{clearPreview();clearMarker()},{once:true});
    }
  };
}
