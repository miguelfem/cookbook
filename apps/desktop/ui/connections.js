'use strict';
(() => {
  const $ = id => document.getElementById(id);
  const invoke = window.__TAURI__.core.invoke;
  const call = request => invoke('owner_request', {request});
  const element = (tag, text) => { const node=document.createElement(tag); if(text!==undefined)node.textContent=text; return node; };
  const run = async action => { try { await action(); } catch(error) { $('status').textContent=typeof error==='string'?error:error.message; } };
  const button = (text, action) => { const node=element('button',text);node.type='button';node.addEventListener('click',()=>run(action));return node; };
  let next=null;
  let accountNext=null;
  async function accounts(after=null){
    const result=await call({action:'list_accounts',after});
    $('saved-accounts').replaceChildren();
    if(!result.accounts.length)$('saved-accounts').append(element('p','No accounts saved yet. Open Connect a service, or approve a connection prepared by your agent.'));
    for(const account of result.accounts){
      const card=element('article');card.className='review';
      card.append(element('h3',account.label),element('p',`${account.auth_kind==='oauth'?'Browser authorization':'Protected service credential'} · ${account.endpoint}`),element('p',account.enabled?'Saved for separately reviewed operations':'Disconnected'));
      if(account.enabled)card.append(button('Disconnect account',async()=>{
        if(!await window.__TAURI__.dialog.confirm(`Disconnect ${account.label}? Every tool using this account will stop before its next call. Calls already sent may still finish. Provider-side access can also be revoked in the service’s account settings.`))return;
        const receipt=await call({action:'revoke_account',account_id:account.account_id,revision:account.revision});
        await accounts();await load();$('workspace-tools').replaceChildren();
        window.dispatchEvent(new Event('cookbook-connections-changed'));
        $('status').textContent=receipt.credential_removed ? 'Account disconnected and its local key removed. Its tool grants are disabled.' : 'Account disconnected and its tool grants disabled. The local key could not be removed; check the credential file before sharing or disposing of it.';
      }));
      $('saved-accounts').append(card);
    }
    accountNext=result.next_cursor;$('accounts-next').hidden=!accountNext;
  }
  async function load(after=null) {
    const result=await call({action:'list_library',after});
    $('library-tools').replaceChildren();
    if(!result.connections.length)$('library-tools').append(element('p','No tools saved yet. Approve an access request for all projects, or share an existing connection below.'));
    for(const tool of result.connections){
      const card=element('article');card.className='review';
      card.append(element('h3',tool.label),element('p',`${tool.tool||tool.kind} · ${tool.endpoint}`),element('p',tool.enabled?(tool.authenticated?'Available to all projects · key protected':'Available to all projects · no account needed'):'Disabled · unavailable for new calls'));
      if(tool.enabled)card.append(button('Disable in all projects',async()=>{
        if(!await window.__TAURI__.dialog.confirm(`Disable ${tool.label} in all projects? Existing runs will stop before their next use of this connection. Calls already sent may still finish.`))return;
        await call({action:'disable_tool',library_id:tool.library_id,revision:tool.revision});await load();$('workspace-tools').replaceChildren();
        $('status').textContent='Tool disabled in all projects. Any stored account credential remains protected.';
      }));
      $('library-tools').append(card);
    }
    next=result.next_cursor;$('library-next').hidden=!next;
  }
  async function workspace(){
    const status=await invoke('desktop_status');
    if(!status.assignment_id)throw Error('Open a workspace on Connect your agent first.');
    const result=await call({action:'list_bindings',assignment:status.assignment_id});
    $('workspace-tools').replaceChildren();
    if(!result.bindings.length)$('workspace-tools').append(element('p','This workspace has no approved connections.'));
    for(const item of result.bindings){
      const card=element('article');card.className='review';
      card.append(element('h3',item.connection.label),element('p',item.name));
      const details=element('details');details.append(element('summary','Review this exact tool and permissions'),element('pre',JSON.stringify(item.connection,null,2)));card.append(details);
      card.append(button('Make available to all projects',async()=>{
        if(!await window.__TAURI__.dialog.confirm(`Allow ${item.connection.label} in every existing and future CookBook project, with these exact permissions and its stored account?`))return;
        await call({action:'save_tool',assignment:status.assignment_id,name:item.name,revision:item.revision});await load();
        $('status').textContent='Tool saved for all projects. Existing credentials stay protected; no key entry is needed.';
      }));$('workspace-tools').append(card);
    }
    if(result.truncated)$('workspace-tools').append(element('p','Only the first bounded set is shown. Use the owner CLI to review another named connection.'));
  }
  $('library-refresh').addEventListener('click',()=>run(()=>load()));
  $('accounts-refresh').addEventListener('click',()=>run(()=>accounts()));
  $('accounts-next').addEventListener('click',()=>run(()=>accounts(accountNext)));
  $('library-next').addEventListener('click',()=>run(()=>load(next)));
  $('workspace-tools-refresh').addEventListener('click',()=>run(workspace));
  window.addEventListener('cookbook-connections-changed',()=>run(async()=>{await load();await accounts();}));
  run(()=>load());
  run(()=>accounts());
})();
