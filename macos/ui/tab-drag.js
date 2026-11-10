import {moveAccount} from './tab-order.js';

// Pointer dragging avoids HTML file-drop handling in the native WebView.
// The real buttons stay mounted; CSS order previews the eventual placement.
export function createTabDrag({getAccounts,onCommit,onDetach,onExternalMove,onExternalEnd,onDragState,onRender,onError=console.error}){
  let drag=null,frame=0,deferred=false,suppressClick=null;
  const tabButtons=list=>[...list.querySelectorAll('[data-account-tab]')];
  function finish(commit=false,position=null){
    if(!drag)return;
    const current=drag;drag=null;cancelAnimationFrame(frame);
    current.ghost?.remove();current.button.classList.remove('tab-placeholder');
    onExternalEnd?.();onDragState?.(false);
    current.list.classList.remove('is-reordering');document.body.classList.remove('tab-reordering');
    for(const tab of tabButtons(current.list)){tab.style.order='';for(const animation of tab.getAnimations())animation.cancel()}
    try{if(current.button.hasPointerCapture(current.pointerId))current.button.releasePointerCapture(current.pointerId)}catch{}
    if(current.started){
      const suppressed={};suppressClick=suppressed;
      // Only suppress the click synthesized by this pointerup. Replacing the
      // buttons can retarget that click, or prevent it from being fired at all.
      setTimeout(()=>{if(suppressClick===suppressed)suppressClick=null},0);
      try{if(commit==='detach')Promise.resolve(onDetach(current.id,position)).catch(onError);else if(commit)onCommit(current.preview.map(account=>account.id))}catch(error){onError(error)}
      deferred=false;onRender(current.id);
    }else if(deferred){deferred=false;setTimeout(()=>onRender(),0)}
  }
  function previewOrder(){
    if(!drag?.started)return;
    const {list,button}=drag,rect=list.getBoundingClientRect();
    drag.detaching=!!onDetach&&(drag.y<rect.top-20||drag.y>rect.bottom+30||drag.x<rect.left-35||drag.x>rect.right+35);
    drag.ghost.classList.toggle('is-detaching',drag.detaching);
    if(drag.detaching){
      drag.ghost.style.left=`${drag.x-drag.offset}px`;drag.ghost.style.top=`${drag.y-drag.height/2}px`;onExternalMove?.({clientX:drag.x,clientY:drag.y});return;
    }
    onExternalMove?.(null);
    drag.ghost.style.top=`${drag.top}px`;
    const center=drag.x-drag.offset+drag.width/2;
    const index=tabButtons(list).filter(tab=>tab!==button&&center>rect.left+tab.offsetLeft-list.scrollLeft+tab.offsetWidth/2).length;
    const next=moveAccount(drag.preview,drag.id,index);
    if(next.some((account,i)=>account.id!==drag.preview[i].id)){
      const buttons=tabButtons(list),before=new Map(buttons.map(tab=>[tab,tab.getBoundingClientRect().left]));
      for(const tab of buttons)for(const animation of tab.getAnimations())animation.cancel();
      drag.preview=next;const positions=new Map(next.map((account,i)=>[account.id,i]));
      for(const tab of buttons)tab.style.order=positions.get(tab.dataset.accountTab);
      for(const tab of buttons){
        if(tab===button)continue;
        const delta=before.get(tab)-tab.getBoundingClientRect().left;
        if(delta&&!matchMedia('(prefers-reduced-motion: reduce)').matches)tab.animate([{transform:`translateX(${delta}px)`},{transform:'translateX(0)'}],{duration:150,easing:'ease-out'});
      }
    }
    drag.ghost.style.left=`${Math.min(Math.max(drag.x-drag.offset,rect.left),Math.max(rect.left,rect.right-drag.width))}px`;
  }
  function scrollFrame(now=performance.now()){
    if(!drag?.started)return;
    if(!drag.button.isConnected){finish(false);return}
    const elapsed=drag.scrollTime===null?0:Math.min(40,Math.max(0,now-drag.scrollTime));drag.scrollTime=now;
    const rect=drag.list.getBoundingClientRect(),edge=38;
    const speed=drag.detaching?0:drag.x<rect.left+edge?-Math.min(18,(rect.left+edge-drag.x)/2):drag.x>rect.right-edge?Math.min(18,(drag.x-rect.right+edge)/2):0;
    if(speed&&elapsed){drag.list.scrollLeft+=speed*elapsed/(1000/60);previewOrder()}
    frame=requestAnimationFrame(scrollFrame);
  }
  document.addEventListener('pointerdown',event=>{
    const button=event.target.closest?.('[data-account-tab]'),accounts=getAccounts();
    if(!button||event.button!==0||(!accounts.length)||(accounts.length<2&&!onDetach)||document.querySelector('dialog[open]'))return;
    if(drag)finish(false);
    const rect=button.getBoundingClientRect(),list=button.closest('.account-tabs');
    drag={id:button.dataset.accountTab,button,list,pointerId:event.pointerId,startX:event.clientX,startY:event.clientY,x:event.clientX,y:event.clientY,offset:event.clientX-rect.left,width:rect.width,top:rect.top,height:rect.height,preview:[...accounts],started:false,scrollTime:null};
    try{button.setPointerCapture(event.pointerId)}catch{}
  });
  document.addEventListener('pointermove',event=>{
    if(!drag||event.pointerId!==drag.pointerId)return;
    if(!drag.button.isConnected){finish(false);return}
    drag.x=event.clientX;drag.y=event.clientY;
    if(!drag.started){
      if(Math.hypot(drag.x-drag.startX,drag.y-drag.startY)<6)return;
      drag.started=true;
      onDragState?.(true);
      const ghost=drag.button.cloneNode(true);ghost.classList.add('tab-drag-preview');ghost.removeAttribute('data-account-tab');ghost.removeAttribute('role');ghost.removeAttribute('title');ghost.setAttribute('aria-hidden','true');ghost.tabIndex=-1;
      Object.assign(ghost.style,{left:`${drag.x-drag.offset}px`,top:`${drag.top}px`,width:`${drag.width}px`,height:`${drag.height}px`});
      document.body.append(ghost);drag.ghost=ghost;drag.button.classList.add('tab-placeholder');drag.list.classList.add('is-reordering');document.body.classList.add('tab-reordering');
      scrollFrame();
    }
    event.preventDefault();previewOrder();
  },{passive:false});
  document.addEventListener('pointerup',event=>{
    if(!drag||event.pointerId!==drag.pointerId)return;
    const rect=drag.list.getBoundingClientRect();
    const outside=event.clientY<rect.top-20||event.clientY>rect.bottom+30||event.clientX<rect.left-35||event.clientX>rect.right+35;
    finish(drag.started&&(outside&&onDetach?'detach':!outside),{x:event.screenX-drag.offset,y:event.screenY-drag.height/2,release:{clientX:event.clientX,clientY:event.clientY}});
  });
  document.addEventListener('pointercancel',event=>{if(drag?.pointerId===event.pointerId)finish(false)});
  document.addEventListener('lostpointercapture',event=>{if(drag?.pointerId===event.pointerId)finish(false)});
  window.addEventListener('blur',()=>finish(false));
  window.addEventListener('resize',()=>finish(false));
  document.addEventListener('visibilitychange',()=>{if(document.hidden)finish(false)});
  document.addEventListener('click',event=>{
    const suppressed=suppressClick;suppressClick=null;
    if(suppressed&&event.detail!==0){event.preventDefault();event.stopImmediatePropagation()}
  },true);
  document.addEventListener('keydown',event=>{
    if(drag&&event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();finish(false);return}
    const tab=event.target.closest?.('[data-account-tab]');
    if(!tab||!(event.ctrlKey||event.metaKey)||!event.shiftKey||!['ArrowLeft','ArrowRight'].includes(event.key))return;
    event.preventDefault();event.stopImmediatePropagation();finish(false);
    const accounts=getAccounts(),index=accounts.findIndex(account=>account.id===tab.dataset.accountTab);
    const next=moveAccount(accounts,tab.dataset.accountTab,index+(event.key==='ArrowRight'?1:-1));
    try{onCommit(next.map(account=>account.id))}catch(error){onError(error)}onRender(tab.dataset.accountTab);
  },true);
  return {isInteracting:()=>!!drag,deferRender:()=>{deferred=true},cancel:()=>finish(false)};
}
