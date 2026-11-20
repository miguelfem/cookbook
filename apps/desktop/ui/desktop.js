'use strict';
(() => {
  window.aprConfirm = message => window.__TAURI__.dialog.confirm(message, {title:'CookBook APR',kind:'warning'});
  const invoke = window.__TAURI__.core.invoke;
  window.aprDesktopCall = async (action, args, detail) => {
    try { return await invoke('scoped_call', {action, args, detail}); }
    catch (error) {
      if (error instanceof Error && error.message) throw error;
      throw new Error(typeof error === 'string' && error.trim() ? error : 'Local request failed. Open Setup to check the service.');
    }
  };
  window.aprOpenSetup=()=>invoke('open_setup');
  const button = document.createElement('button'); button.textContent = 'Settings & access';
  button.type='button'; button.addEventListener('click',()=>invoke('open_setup'));
  (document.querySelector('.header-actions') || document.querySelector('header')).append(button);
  document.querySelectorAll('[data-desktop]').forEach(element => { element.hidden = false; });
  invoke('desktop_status').then(status=>{
    document.getElementById('connection').textContent=status.selected ? `This device · ${status.device}` : 'Connect your agent in Settings';
  });
  window.__TAURI__.event.listen('cookbook:update-available', event => {
    const info = event.payload?.available;
    if (info?.version) { button.textContent = 'Update available · Setup'; button.title = `Review version ${info.version}`; }
  });
  window.__TAURI__.event.listen('cookbook:changed',()=>{window.location.reload();});
})();
