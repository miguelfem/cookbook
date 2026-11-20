'use strict';
(() => {
  const $=id=>document.getElementById(id), invoke=window.__TAURI__.core.invoke;
  const call=request=>invoke('owner_request',{request});
  let assignments=[],selected='',plan=null,agentPlan=null,agentPreviewGeneration=0,agentPreviewPending=false,discovery=null,discoveryAccountKey=null;
  const run=async fn=>{ $('status').textContent='';try{await fn();}catch(e){$('status').textContent=typeof e==='string'?e:e.message;} };
  const element=(tag,text)=>{const el=document.createElement(tag);if(text!==undefined)el.textContent=text;return el;};
  const button=(label,fn)=>{const b=element('button',label);b.type='button';b.addEventListener('click',()=>run(fn));return b;};
  const assignment=()=>{if(!selected)throw Error('Open a recipe workspace on the Connect your agent page first.');return selected;};
  async function savedAccounts(connection,includeReview=false){
    if(connection.auth.kind==='none')return [];
    const accounts=[],seen=new Set();let after=null;
    do{
      const page=await call({action:'list_accounts',after,connection});
      accounts.push(...page.accounts.filter(account=>account.enabled&&(account.compatible||(includeReview&&account.review_required))));
      after=page.next_cursor;
      if(after&&(seen.has(after)||seen.size>=50))throw Error('Account inventory is incomplete. Refresh or review accounts before approving.');
      if(after)seen.add(after);
    }while(after);
    return accounts;
  }
  async function load(){
    const [data,status]=await Promise.all([call({action:'list_assignments'}),invoke('desktop_status')]);
    assignments=data.assignments;selected=status.assignment_id||selected;
    $('assignments').replaceChildren(element('option','Select a workspace'));
    $('assignments').firstChild.value='';
    for(const a of assignments){if(a.state!=='active')continue;const o=element('option',`${a.name} · ${a.root}`);o.value=a.assignment_id;$('assignments').append(o);}
    $('assignments').value=selected;
    const current=assignments.find(a=>a.assignment_id===selected);if(current)$('project-root').value=current.root;
    if(selected)await requests();
  }
  async function requests(after=null){
    const requestAssignment=assignment();
    const data=await call({action:'list_approvals',assignment:requestAssignment,after});$('requests').replaceChildren();
    const pending=data.requests.filter(r=>r.state==='pending');
    if(!pending.length)$('requests').append(element('p',after||data.next_cursor?'No pending access requests on this page.':'No pending access requests.'));
    for(const r of pending){
      const card=element('article');card.className='review';card.append(element('h3',r.name),element('p',r.rationale));
      const cfg=r.connection;
      const saved=await savedAccounts(cfg,true);
      card.append(element('p',`${cfg.mcp?'MCP · '+(cfg.mcp.tool?.name||cfg.mcp.feature?.uri||cfg.mcp.feature?.name):cfg.method} · ${cfg.endpoint}`));
      const native=cfg.mcp?.transport.kind==='stdio';
      if(native)card.append(element('p','Local adapter: this process calls the service using the credential below. This is not a browser connection to the service. The command and its dependencies have your operating-system access.'));
      if(!cfg.metering)card.append(element('p','Cost coverage: unknown. Ask your agent to analyze the selected payloads and pricing terms.'));
      const details=element('details'),summary=element('summary','Review exact tool, payload constraints & cost rules'),pre=element('pre',JSON.stringify(cfg,null,2));details.append(summary,pre);card.append(details);
      const credentialLabel=cfg.credential_label||(native?'Credential required by this local adapter':cfg.auth.kind==='header'?`API key for ${cfg.auth.name}`:'Bearer credential for this exact service endpoint');
      const label=element('label',credentialLabel),key=element('input');key.type='password';key.autocomplete='new-password';label.append(key);if(!['none','oauth'].includes(cfg.auth.kind)){
        card.append(label);
        if(native)card.append(element('p',`Protected environment injection: ${cfg.mcp.transport.credential_env}. Use the credential required by the adapter; a service's management token and project API key may be different.`));
        if(!cfg.credential_label)card.append(element('p','This older proposal does not name the exact credential type. Check the agent’s explanation before entering a key.'));
      }
      if(cfg.auth.kind==='oauth')card.append(element('p','Browser authorization · '+cfg.auth.config.issuer+' · scopes: '+(cfg.auth.config.scopes.join(', ')||'Provider default (review in browser)')));
      const availability=element('select');
      for(const [value,text] of [['workspace','Only this workspace'],['all','All projects — share this exact tool']]){const option=element('option',text);option.value=value;availability.append(option);}
      const scopeLabel=element('label','Available to');scopeLabel.append(availability);card.append(scopeLabel);
      card.append(element('p','Your account is saved securely for future approvals. Sharing this tool is a separate choice. “All projects” includes future projects; different operations still require review.'));
      const credential=element('select'),newKey=element('option',cfg.auth.kind==='oauth'?'Connect an account in the browser':'Enter a new credential');newKey.value='';credential.append(newKey);
      for(const account of saved){const option=element('option',(account.compatible?'Use saved account: ':'Review updated adapter: ')+account.label);option.value=account.account_id;option.dataset.revision=account.revision;option.dataset.reviewRequired=account.compatible?'false':'true';credential.append(option);}
      let accountReview=null,reviewGeneration=0;
      const reviewPanel=element('div'),reviewBody=element('div'),reviewConsent=element('label'),reviewCheck=element('input');
      reviewPanel.hidden=true;reviewCheck.type='checkbox';reviewConsent.className='consent';reviewConsent.append(reviewCheck,document.createTextNode('I reviewed the changed code pins, command, destinations and operation permissions, and trust this updated adapter with the selected saved account.'));
      const reviewButton=button('Review adapter update',async()=>{
        accountReview=null;reviewCheck.checked=false;reviewBody.replaceChildren();
        const option=credential.selectedOptions[0];if(option?.dataset.reviewRequired!=='true')throw Error('Select an adapter update to review.');
        const generation=++reviewGeneration,accountId=credential.value;
        const value=await call({action:'preview_account_reuse',request_id:r.request_id,digest:r.digest,account_id:accountId,revision:Number(option.dataset.revision)});
        if(generation!==reviewGeneration||credential.value!==accountId)return;
        if(typeof value.review_digest!=='string')throw Error('Account review is incomplete. Refresh and try again.');
        accountReview={account_id:accountId,revision:Number(option.dataset.revision),review_digest:value.review_digest};
        reviewBody.append(element('h4','Saved account → updated adapter'),element('p','Review the old and new code pins, launch arguments, destination constraints and operation schemas below. Native code has OS access. A local command can make destinations opaque; inspect its source and dependencies before approving.'),element('pre',JSON.stringify(value,null,2)));
      });
      reviewPanel.append(element('p','This account belongs to an earlier adapter revision. Reuse requires a separate review; your key stays in the credential store.'),reviewButton,reviewBody,reviewConsent);
      if(saved.some(account=>account.review_required&&!account.compatible))card.append(reviewPanel);
      if(cfg.auth.kind!=='none'&&credential.options.length>1){const source=element('label','Account');source.append(credential);card.append(source);credential.addEventListener('change',()=>{label.hidden=Boolean(credential.value);key.value='';accountReview=null;reviewGeneration++;reviewCheck.checked=false;check.checked=false;reviewBody.replaceChildren();reviewPanel.hidden=credential.selectedOptions[0]?.dataset.reviewRequired!=='true';});}
      const exact=saved.filter(account=>account.compatible);
      if(exact.length===1){credential.value=exact[0].account_id;label.hidden=true;}
      const consent=element('label'),check=element('input');check.type='checkbox';consent.className='consent';consent.append(check,document.createTextNode('I approve this destination, action, input constraints, and cost assumptions for the projects selected above, including trusted OS access and token access for any local MCP command.'));card.append(consent);
      const actions=element('div');actions.className='actions';
      const approve=button('Approve access',async()=>{
        if(!check.checked)throw Error('Review and confirm this specific access first.');
        const value=key.value;key.value='';approve.disabled=true;
        const allProjects=availability.value==='all';
        const selectedCredential=credential.selectedOptions[0];
        const options={all_projects:allProjects,saved_account:credential.value?{account_id:credential.value,revision:Number(selectedCredential.dataset.revision)}:null};
        if(selectedCredential?.dataset.reviewRequired==='true'){
          if(!accountReview||!reviewCheck.checked||accountReview.account_id!==credential.value){approve.disabled=false;throw Error('Review this adapter update and confirm saved-account reuse first.');}
          options.saved_account=null;options.reviewed_account_reuse=accountReview;
        }
        try{if(cfg.auth.kind==='oauth'&&!credential.value){
            const flow=await invoke('oauth_begin',{request:{request_id:r.request_id,digest:r.digest,consent:true,all_projects:allProjects}});
            const cancel=button('Cancel browser sign-in',()=>invoke('oauth_cancel',{flowId:flow.flow_id}));actions.append(cancel);
            try{await new Promise((resolve,reject)=>{const until=Date.now()+310000;const poll=async()=>{try{const status=await invoke('oauth_status',{flowId:flow.flow_id});if(status.state==='connected'){resolve();return;}if(status.state==='stopped'||Date.now()>until){reject(Error(status.message||'Authorization expired'));return;}setTimeout(poll,1000);}catch(e){reject(e);}};poll();});}finally{cancel.remove();}
          }else{await call({action:'approve',request_id:r.request_id,digest:r.digest,secret:credential.value?null:value||null,options});}await requests();window.dispatchEvent(new Event('cookbook-connections-changed'));$('status').textContent=allProjects?'Account saved and this exact tool shared with all projects. Existing blocked runs need an explicit rebind.':'Account saved; this operation is approved only for this workspace. Other operations require their own approval. Existing blocked runs need an explicit rebind.';}finally{approve.disabled=false;}
      });
      if(r.rate_only){approve.hidden=true;scopeLabel.hidden=true;availability.value='workspace';label.hidden=true;credential.closest('label')?.setAttribute('hidden','');consent.lastChild.textContent='I approve this rate-only change. Existing access and project scope remain unchanged.';}
      if(r.rate_only)actions.append(button('Approve rate-only change',async()=>{if(!check.checked)throw Error('Review and confirm this change first.');await call({action:'approve_price_review',request_id:r.request_id,digest:r.digest});await requests();}));actions.append(approve,button('Decline',async()=>{await call({action:'reject',request_id:r.request_id,digest:r.digest});await requests();}));card.append(actions);$('requests').append(card);
    }
    if(after)$('requests').append(button('First requests',()=>requests()));
    if(data.next_cursor)$('requests').append(button('Next requests',()=>requests(data.next_cursor)));
    else if(data.truncated)$('requests').append(element('p','More requests exist. Refresh with the latest service version to page through them.'));
  }
  const clearAgentPreview=()=>{agentPreviewGeneration++;agentPlan=null;$('apply-agent').disabled=true;$('agent-result').textContent='';$('agent-settings').textContent='';};
  $('preview-agent').addEventListener('click',()=>run(async()=>{
    if(agentPreviewPending)return;
    agentPreviewPending=true;$('preview-agent').disabled=true;
    try{
    clearAgentPreview();const generation=agentPreviewGeneration;
    const value=await call({action:'preview_agent',client:$('global-client').value,trust_apr_run:$('global-client').value==='codex'&&Boolean($('trust-apr-run').checked)});
    if(generation!==agentPreviewGeneration)return;
    agentPlan=value;$('agent-settings').textContent=JSON.stringify({config:agentPlan.config_path,skill:agentPlan.skill_path,connection:agentPlan.server_entry,changes:agentPlan.changes},null,2);$('agent-preview').open=true;$('apply-agent').disabled=false;
    $('agent-result').textContent=agentPlan.trust_apr_run?'Review these settings, then connect. Codex will trust recipe runs in every project using this user-level CookBook connection; CookBook still enforces workspace permissions and execution grants. Other Codex approvals stay unchanged.':'Review these settings, then connect. Your agent’s existing approval rules stay unchanged.';
    }finally{agentPreviewPending=false;$('preview-agent').disabled=false;}
  }));
  $('global-client').addEventListener('change',()=>{clearAgentPreview();const codex=$('global-client').value==='codex';$('run-trust-choice').hidden=!codex;if(!codex)$('trust-apr-run').checked=false;});
  $('trust-apr-run').addEventListener('change',clearAgentPreview);
  $('apply-agent').addEventListener('click',()=>run(async()=>{if(!agentPlan)throw Error('Preview the connection first.');await call({action:'apply_agent',plan_id:agentPlan.plan_id});agentPlan=null;$('apply-agent').disabled=true;$('agent-result').textContent='Connection installed. Open a new agent task in any project and ask it to use CookBook. Your agent may need a restart or its own trust approval.';}));
  $('refresh-workspaces').addEventListener('click',()=>run(load));
  $('choose-root').addEventListener('click',()=>run(async()=>{const value=await window.__TAURI__.dialog.open({directory:true,multiple:false});if(value)$('project-root').value=value;}));
  $('project-form').addEventListener('submit',e=>{e.preventDefault();run(async()=>{const value=await call({action:'create_project',root:$('project-root').value,name:$('assignment-name').value});selected=value.assignment_id;await load();$('status').textContent='Manual workspace created. Open it here, or use the advanced project-only connection.';});});
  $('select-assignment').addEventListener('click',()=>run(async()=>{selected=$('assignments').value;await call({action:'select_assignment',assignment:assignment()});await load();}));
  $('refresh').addEventListener('click',()=>run(requests));
  $('preview-client').addEventListener('click',()=>run(async()=>{plan=await call({action:'preview_client',root:$('project-root').value,client:$('client').value});$('client-result').textContent=JSON.stringify(plan,null,2);$('client-preview').open=true;$('apply-client').disabled=false;}));
  for(const id of ['project-root','client'])$(id).addEventListener('change',()=>{plan=null;$('apply-client').disabled=true;});
  $('apply-client').addEventListener('click',()=>run(async()=>{if(!plan)throw Error('Preview current settings first.');const data=await call({action:'apply_client',plan_id:plan.plan_id});plan=null;$('apply-client').disabled=true;$('client-result').textContent=JSON.stringify(data,null,2);}));
  $('doctor-client').addEventListener('click',()=>run(async()=>{$('client-result').textContent=JSON.stringify(await call({action:'doctor_client',root:$('project-root').value,client:$('client').value}),null,2);$('client-preview').open=true;}));
  $('transport').addEventListener('change',()=>{$('native-fields').hidden=$('transport').value!=='stdio';$('endpoint-label').hidden=$('transport').value==='stdio';discovery=null;$('discovered-tools').replaceChildren();});
  $('mcp-auth').addEventListener('change',()=>{
    $('mcp-secret-label').hidden=$('mcp-auth').value!=='key';
    $('mcp-secret').value='';$('mcp-account-label').hidden=true;
    discoveryAccountKey=null;discovery=null;$('discovered-tools').replaceChildren();
  });
  let connecting=false;
  $('mcp-form').addEventListener('submit',e=>{e.preventDefault();if(connecting)return;run(async()=>{
    connecting=true;$('mcp-connect').disabled=true;
    try {
    discovery=null;$('discovered-tools').replaceChildren();
    const native=$('transport').value==='stdio';if(native&&!$('native-trust').checked)throw Error('Review and trust the exact local command before discovery.');
    const mode=$('mcp-auth').value;
    if(native&&mode==='browser')throw Error('Local servers use their own installed software. Choose API key or No account needed in Advanced service settings.');
    let transport={kind:'http'};
    if(native){const pin=await call({action:'pin_command',executable:$('executable').value});transport={kind:'stdio',executable:pin.executable,sha256:pin.sha256,args:JSON.parse($('command-args').value),credential_env:$('credential-env').value||null};}
    const key=$('mcp-secret').value;$('mcp-secret').value='';
    const keyFormat=$('mcp-key-header').value||'bearer';
    const keyAuth=keyFormat==='bearer'?{kind:'bearer',credential_ref:'pending'}:{kind:'header',name:keyFormat,credential_ref:'pending'};
    const automatic_protocol=$('protocol').value==='auto';
    const cfg={label:$('mcp-label').value,endpoint:native?'stdio:local':$('endpoint').value.trim(),method:'POST',auth:mode==='key'?keyAuth:{kind:'none'},input_schema:{type:'object'},output_schema:{type:'object'},timeout_ms:30000,max_response_bytes:65536,mcp:{protocol:automatic_protocol?'2026-07-28':$('protocol').value,transport,tool:null}};
    const kind=$('mcp-feature').value;let data;
    if(!native&&mode!=='none'){
      let auth;
      if(mode==='browser'){
        if($('mcp-use-oauth').checked){
          auth=JSON.parse($('oauth-config').textContent);if(auth.resource!==cfg.endpoint)throw Error('The reviewed sign-in configuration belongs to another service address.');
        }else{
          $('status').textContent='Finding this service’s browser sign-in…';
          auth=await call({action:'discover_oauth',resource:cfg.endpoint,metadata_url:null,client_id:'',scopes:[]});
        }
        cfg.auth={kind:'oauth',credential_ref:'pending',config:auth};
      }
      const accounts=await savedAccounts(cfg), accountSelect=$('mcp-account');
      const accountKey=JSON.stringify({endpoint:cfg.endpoint,transport:cfg.mcp.transport,auth:cfg.auth});
      const previous=discoveryAccountKey===accountKey?accountSelect.value:undefined;
      accountSelect.replaceChildren(element('option',mode==='browser'?'Sign in to another account':'Save a new API key'));accountSelect.firstChild.value='';
      for(const account of accounts){const option=element('option','Use saved account: '+account.label);option.value=account.account_id;accountSelect.append(option);}
      accountSelect.value=key?'':previous!==undefined&&(previous===''||accounts.some(a=>a.account_id===previous))?previous:accounts.length===1?accounts[0].account_id:'';
      $('mcp-account-label').hidden=accounts.length===0;discoveryAccountKey=accountKey;
      if(accounts.length>1&&previous===undefined&&!key){$('status').textContent='Choose the saved account to inspect, or connect another account, then select Connect service again.';return;}
      const account=accounts.find(a=>a.account_id===accountSelect.value);
      if(mode==='key'&&!account&&!key)throw Error('Paste the API key into the field, or choose a saved account.');
      const review=account?`Use saved account ${account.label} for ${cfg.endpoint}? This reads tool metadata and does not approve any tool execution.`:mode==='browser'?`Connect and securely save an account for ${auth.resource} through ${auth.issuer}, scopes ${auth.scopes.join(', ')||'provider default (review in browser)'}? This reads tool metadata and does not approve any tool execution.`:`Securely save this API key for ${cfg.endpoint}? It will be sent only to this exact service address, using ${keyFormat}. This reads tool metadata and does not approve any tool execution.`;
      if(!await window.__TAURI__.dialog.confirm(review))return;
      const saved_account=account?{account_id:account.account_id,revision:account.revision}:null;
      let receipt;
      if(mode==='key'){
        $('status').textContent='Saving the account and reading available tools…';
        receipt=await call({action:'connect_mcp_account',connection:cfg,secret:key||null,saved_account,kind,automatic_protocol});
      }else{
      const flow=await invoke('oauth_discover_begin',{request:{connection:cfg,kind,consent:true,saved_account,automatic_protocol}});
      const cancel=button('Cancel metadata sign-in',()=>invoke('oauth_cancel',{flowId:flow.flow_id}));$('discovered-tools').replaceChildren(cancel);
      $('status').textContent='Complete sign-in in your browser. CookBook will save the account and read its available tools.';
      try{receipt=await new Promise((resolve,reject)=>{const until=Date.now()+310000;const poll=async()=>{try{const status=await invoke('oauth_status',{flowId:flow.flow_id});if(status.state==='connected'){resolve(status.receipt);return;}if(status.state==='stopped'||Date.now()>until){reject(Error(status.message||'Authorization expired'));return;}setTimeout(poll,1000);}catch(e){reject(e);}};poll();});}
      catch(error){await invoke('oauth_cancel',{flowId:flow.flow_id}).catch(()=>{});throw error;}
      finally{$('discovered-tools').replaceChildren();}
      }
      window.dispatchEvent(new Event('cookbook-connections-changed'));
      if(receipt.account?.account_id){
        if(![...accountSelect.options].some(option=>option.value===receipt.account.account_id)){
          const option=element('option','Use saved account: '+cfg.label);option.value=receipt.account.account_id;accountSelect.append(option);
        }
        accountSelect.value=receipt.account.account_id;$('mcp-account-label').hidden=false;
      }
      if(receipt.discovery_error){
        $('status').textContent=`Account saved. Tool discovery failed: ${receipt.discovery_error} Retry discovery with the same configuration; no execution grant has been created.`;
        return;
      }
      data=receipt.discovery;
    }else{data=await call({action:'discover_mcp',connection:cfg,secret:key||null,native_trust:native,kind,automatic_protocol});}
    if(data.protocol)cfg.mcp.protocol=data.protocol;
    discovery=cfg;$('discovered-tools').replaceChildren();
    const entries=data[kind]||data.items||[];
    for(const feature of entries){const row=element('article');row.className='review';row.append(element('h3',feature.name||feature.uri),button('Select for recipe connection',async()=>{
      const connection=JSON.parse(JSON.stringify(discovery));
      if(kind==='tools'){connection.mcp.tool=feature;connection.input_schema=feature.input_schema;}
      else if(kind==='resources'){connection.mcp.feature={kind:'resource',uri:feature.uri};connection.input_schema={type:'object',additionalProperties:false};}
      else{connection.mcp.feature={kind:'prompt',name:feature.name,arguments:feature.arguments||[]};connection.input_schema={type:'object',properties:Object.fromEntries((feature.arguments||[]).map(a=>[a.name,{type:'string'}])),required:(feature.arguments||[]).filter(a=>a.required).map(a=>a.name),additionalProperties:false};}
      $('connection-json').value=JSON.stringify(connection,null,2);$('binding-name').value=(feature.name||'resource').replace(/[^a-zA-Z0-9_.-]/g,'-');$('rationale').value=`Recipe access to ${feature.name||feature.uri}`;openSettingsPage('proposal');
    }));const details=element('details');details.append(element('summary','Discovered contract'),element('pre',JSON.stringify(feature,null,2)));row.append(details);$('discovered-tools').append(row);}
    if(data.truncated)$('discovered-tools').append(element('p','Discovery is truncated. Narrow this server catalog before granting access.'));
    $('status').textContent=mode==='none'||native?'Available tools loaded. Approve only the operations your recipes need.':'Account saved and available tools loaded. Your agent can request the operations a recipe needs without asking for the key again.';
    } finally {connecting=false;$('mcp-connect').disabled=false;$('mcp-secret').value='';}
  });});
  $('proposal-form').addEventListener('submit',e=>{e.preventDefault();run(async()=>{await call({action:'propose',assignment:assignment(),proposal:{name:$('binding-name').value,request_key:crypto.randomUUID(),rationale:$('rationale').value,connection:JSON.parse($('connection-json').value)}});await requests();openSettingsPage('approvals');});});
  $('revoke').addEventListener('click',()=>run(async()=>{if(!await window.__TAURI__.dialog.confirm('Revoke this connection? Future calls stop; completed external actions are not undone.'))return;await call({action:'revoke',assignment:assignment(),name:$('revoke-name').value});$('status').textContent='Connection revoked.';}));
  $('rebind').addEventListener('click',()=>run(async()=>{if(!await window.__TAURI__.dialog.confirm('Bind this paused run to the current reviewed connection revisions?'))return;const result=await call({action:'rebind',run_id:$('rebind-run').value});$('status').textContent=JSON.stringify(result);}));
  $('stop').addEventListener('click',()=>run(async()=>{if(!await window.__TAURI__.dialog.confirm('Stop the runtime service? Accepted work remains saved; in-flight external effects may still need reconciliation.'))return;await call({action:'stop_service'});$('status').textContent='Stop requested. Work is saved; in-flight effects may still finish. Opening a recipe can restart the service.';}));
  run(load);
  const settingsPages = [...document.querySelectorAll('[data-setup-page]')];
  function settingsPage() {
    const requested = location.hash.slice(1) || 'projects';
    const page = settingsPages.some(node => node.dataset.setupPage === requested) ? requested : 'projects';
    settingsPages.forEach(node => { node.hidden = node.dataset.setupPage !== page; });
    document.querySelectorAll('[data-setup-nav]').forEach(link => {
      if (link.dataset.setupNav === page) {
        link.setAttribute('aria-current','page'); $('setup-title').textContent = link.textContent;
        const group=link.closest('details'); if(group) group.open=true;
      } else link.removeAttribute('aria-current');
    });
    document.title = 'CookBook · ' + $('setup-title').textContent;
    $('setup-title').focus({preventScroll:true}); window.scrollTo({top:0,behavior:'instant'});
  }
  function openSettingsPage(page) { location.hash = page; settingsPage(); }
  if (settingsPages.length) {
    window.addEventListener('hashchange', settingsPage);
    document.querySelector('.skip-link').addEventListener('click', event => { event.preventDefault(); $('setup-title').focus(); });
    settingsPage();
  }
})();
