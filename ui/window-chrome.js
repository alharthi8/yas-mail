const icons={
  minimize:'<path d="M2 6h10"/>',
  maximize:'<rect x="2.5" y="2.5" width="9" height="9"/>',
  restore:'<path d="M4.5 2.5h7v7M2.5 4.5h7v7h-7z"/>',
  close:'<path d="m2.5 2.5 9 9m0-9-9 9"/>'
};
const icon=type=>`<svg viewBox="0 0 14 14" width="12" height="12" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true">${icons[type]}</svg>`;

// The native frame is replaced by controls inside the existing app header.
// Tauri handles dragging/double-clicking through data-tauri-drag-region.
export function createWindowChrome({api=globalThis.__TAURI__?.window,onError=console.error}={}){
  const appWindow=api?.getCurrentWindow();
  let maximized=false;
  const updateButton=()=>{
    const label=maximized?'Restore window':'Maximize window';
    for(const button of document.querySelectorAll('[data-window-action="maximize"]')){
      button.setAttribute('aria-label',label);
      button.title=label;
      button.innerHTML=icon(maximized?'restore':'maximize');
    }
  };
  const refresh=async()=>{
    try{maximized=await appWindow.isMaximized();updateButton()}catch(error){onError(error)}
  };
  return {
    markup(floating=false){
      if(!appWindow)return '';
      return `<div class="window-controls${floating?' window-controls-floating':''}" role="group" aria-label="Window controls" data-tauri-drag-region="false"><button type="button" class="window-button" data-window-action="minimize" title="Minimize window" aria-label="Minimize window">${icon('minimize')}</button><button type="button" class="window-button" data-window-action="maximize" title="${maximized?'Restore window':'Maximize window'}" aria-label="${maximized?'Restore window':'Maximize window'}">${icon(maximized?'restore':'maximize')}</button><button type="button" class="window-button window-close" data-window-action="close" title="Close window" aria-label="Close window">${icon('close')}</button></div>`;
    },
    async init(){
      if(!appWindow)return;
      document.addEventListener('click',async event=>{
        const button=event.target.closest('[data-window-action]');
        if(!button||button.disabled)return;
        button.disabled=true;
        try{
          switch(button.dataset.windowAction){
            case 'minimize':await appWindow.minimize();break;
            case 'maximize':await appWindow.toggleMaximize();await refresh();break;
            case 'close':await appWindow.close();break;
          }
        }catch(error){onError(error)}finally{button.disabled=false}
      });
      await refresh();
      try{
        const unlisten=await appWindow.onResized(refresh);
        window.addEventListener('pagehide',()=>unlisten(),{once:true});
      }catch(error){onError(error)}
    }
  };
}
