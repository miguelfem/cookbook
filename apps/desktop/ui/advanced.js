'use strict';
(() => {
  const $=id=>document.getElementById(id), invoke=window.__TAURI__.core.invoke;
  const call=request=>invoke('owner_request',{request});
  const el=(tag,text)=>{const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e;};
  const selected=async()=>{const s=await invoke('desktop_status');if(!s.assignment_id)throw Error('Choose an assignment first.');return s.assignment_id;};
  const act=async(fn)=>{try{await fn();}catch(e){$('status').textContent=typeof e==='string'?e:(e.message||'Operation failed');}};
  const button=(label,fn)=>{const b=el('button',label);b.type='button';b.onclick=()=>act(fn);return b;};
  const json=id=>JSON.parse($(id).value);
  const local=date=>new Date(date.getTime()-date.getTimezoneOffset()*60000).toISOString().slice(0,16);
  const now=new Date();$('billing-start').value=local(new Date(now.getFullYear(),now.getMonth(),1));$('billing-end').value=local(new Date(now.getFullYear(),now.getMonth()+1,1));
  const period=()=>{const starts_at_ms=new Date($('billing-start').value).getTime(),ends_at_ms=new Date($('billing-end').value).getTime();if(!Number.isSafeInteger(starts_at_ms)||!Number.isSafeInteger(ends_at_ms)||ends_at_ms<=starts_at_ms)throw Error('Choose a valid billing period.');return {starts_at_ms,ends_at_ms};};
  $('budget-json').value=JSON.stringify({currency:'USD',...period(),limit_nanos:'10000000000',per_connection_max_nanos:{}},null,2);
  $('invoice-json').value=JSON.stringify({invoice_id:'',account_label:'',connection_name:'',currency:'USD',...period(),lines:[]},null,2);
  $('billing-load').onclick=()=>act(async()=>{$('billing-result').textContent=JSON.stringify(await call({action:'billing',assignment:await selected(),...period()}),null,2);});
  $('budget-load').onclick=()=>act(async()=>{const result=await call({action:'budget',assignment:await selected()});if(result.policy)$('budget-json').value=JSON.stringify(result.policy,null,2);$('status').textContent=result.policy?'Loaded current dispatch budget.':'No monetary dispatch budget configured.';});
  $('budget-save').onclick=()=>act(async()=>{const policy=json('budget-json');if(!await window.__TAURI__.dialog.confirm('Apply this exact period, currency, limit and per-connection reservation policy? Missing or unknown costs can block recipe execution.'))return;const r=await call({action:'set_budget',assignment:await selected(),policy});$('billing-result').textContent=JSON.stringify(r,null,2);});
  $('invoice-import').onclick=()=>act(async()=>{const invoice=json('invoice-json');if(!await window.__TAURI__.dialog.confirm('Import this invoice as owner-provided billing evidence? Its account, period, currency and attempt matches must be correct.'))return;$('billing-result').textContent=JSON.stringify(await call({action:'import_invoice',assignment:await selected(),invoice}),null,2);});
  $('oauth-discover').onsubmit=e=>{e.preventDefault();act(async()=>{const cfg=await call({action:'discover_oauth',resource:$('oauth-resource').value,metadata_url:$('oauth-metadata').value||null,client_id:$('oauth-client-id').value,scopes:$('oauth-scopes').value.split(/\s+/).filter(Boolean)});$('oauth-config').textContent=JSON.stringify(cfg,null,2);if($('connection-json').value.trim()){const connection=json('connection-json');if(connection.endpoint!==cfg.resource)throw Error('The selected proposal has another resource. Do not reuse this authorization configuration.');connection.auth={kind:'oauth',credential_ref:'pending',config:cfg};$('connection-json').value=JSON.stringify(connection,null,2);}$('status').textContent='Review the issuer, registration method, exact resource and every requested scope. Browser sign-in requires your approval; discovering an account does not approve its tools.';});};
  $('price-source-form').onsubmit=e=>{e.preventDefault();act(async()=>{if(!await window.__TAURI__.dialog.confirm('Allow periodic public JSON requests to this exact price source? Changed rates will require review.'))return;$('price-result').textContent=JSON.stringify(await call({action:'set_price_source',assignment:await selected(),name:$('price-name').value,enabled:$('price-enabled').checked,source:{endpoint:$('price-endpoint').value,pointer:$('price-pointer').value,interval_seconds:Number($('price-interval').value)}}),null,2);});};
  $('price-check').onclick=()=>act(async()=>{$('price-result').textContent=JSON.stringify(await call({action:'check_price_source',assignment:await selected(),name:$('price-name').value}),null,2);});
  $('price-load').onclick=()=>act(async()=>{$('price-result').textContent=JSON.stringify(await call({action:'price_sources',assignment:await selected()}),null,2);});
  let updateVersion=null;
  $('update-preferences-save').onclick=()=>act(async()=>{$('update-result').textContent=JSON.stringify(await call({action:'set_update_preferences',enabled:$('update-auto').checked}),null,2);});
  act(async()=>{$('update-auto').checked=(await call({action:'update_preferences'})).automatic_checks;});
  $('update-check').onclick=()=>act(async()=>{const result=await invoke('update_check');$('update-result').textContent=JSON.stringify(result,null,2);updateVersion=result.available?.version||null;$('update-install').disabled=!updateVersion;});
  $('update-install').onclick=()=>act(async()=>{if(!updateVersion)throw Error('Check for an update first.');if(!await window.__TAURI__.dialog.confirm(`Install publisher update ${updateVersion}? Active recipes must finish first. CookBook will back up local state, pause dispatch, verify the signature and restart.`))return;$('update-install').disabled=true;await invoke('update_install',{version:updateVersion,consent:true});});
  $('maintenance-status').onclick=()=>act(async()=>{$('update-result').textContent=JSON.stringify(await call({action:'maintenance_status'}),null,2);});
  $('maintenance-finish').onclick=()=>act(async()=>{if(!await window.__TAURI__.dialog.confirm('Re-enable recipe execution after the completed or cancelled update? Check that the intended application version is running.'))return;$('update-result').textContent=JSON.stringify(await call({action:'finish_maintenance'}),null,2);});
  async function waits(){
    const assignment=await selected(),result=await call({action:'mcp_interactions',assignment});$('mcp-waits').replaceChildren();
    if(!result.items.length)$('mcp-waits').append(el('p','No MCP input requests are waiting.'));
    for(const item of result.items){
      const box=el('article');box.className='review';box.append(el('h3',`Recipe ${item.run_id} · step ${item.step_id}`));
      const replies=new Map();
      for(const [id,request] of Object.entries(item.input_requests)){
        const part=el('section');part.append(el('h4',request.params?.message||request.method));const p=request.params;
        if(request.method==='sampling/createMessage'){
          part.append(el('p',item.sampling_worker ? `An approved API worker (${item.sampling_worker.connection}) handles this request. Check its recorded status below; this window does not issue inference calls.` : 'Your attached agent must return an actual model result. No model is invoked by this window.'),el('pre',JSON.stringify(request,null,2)));
          const job=(item.sampling_jobs||[]).find(j=>j.input_id===id);
          if(job){part.append(el('p',`Worker: ${job.state} · attempt ${job.intent_id}`));
            if(['uncertain','dispatched'].includes(job.state)){
              part.append(el('p','Do not generate another response until this attempt is reconciled. Enter verified evidence below.'));
              const evidence=el('textarea');evidence.placeholder='What you verified at the provider';evidence.maxLength=2000;
              const output=el('textarea');output.placeholder='Observed provider response JSON (only for observed result)';
              const outcome=el('select');for(const key of ['observed_result','confirmed_no_effect','cancel_uncertain']){const o=el('option',key);o.value=key;outcome.append(o);}
              part.append(evidence,output,outcome,button('Reconcile worker attempt',async()=>{if(!evidence.value.trim())throw Error('Observed evidence is required.');if(!await window.__TAURI__.dialog.confirm('Record this evidence and reconcile the selected provider attempt? This does not undo any charge.'))return;await call({action:'reconcile',request:{run_id:item.run_id,step_instance_id:item.step_id,intent_id:job.intent_id,outcome:outcome.value,evidence:evidence.value,output:outcome.value==='observed_result'?JSON.parse(output.value):null}});await waits();}));
            }
          }
        } else if(p.mode==='url'){
          part.append(el('p',`External destination: ${p.url}`),el('p','Open this exact address in your own browser only after reviewing its domain. Completing an external page does not itself prove the action succeeded.'));
          // Deliberately no auto-navigation or fetch. Selecting accept is explicit
          // protocol input; the server must verify any external completion itself.
          const choose=el('select');for(const [value,label] of [['','Choose a response'],['accept','I completed the external step'],['decline','Decline'],['cancel','Cancel']]){const o=el('option',label);o.value=value;choose.append(o);}part.append(choose);replies.set(id,()=>{if(!choose.value)throw Error('Choose a response for each request.');return {action:choose.value};});
        } else {
          const fields=[];const schema=p.requestedSchema;
          for(const [name,def] of Object.entries(schema.properties||{})){
            const label=el('label',def.title||name);let control;
            if(Array.isArray(def.enum)){control=el('select');const blank=el('option','Select');blank.value='';control.append(blank);for(const value of def.enum){const o=el('option',String(value));o.value=JSON.stringify(value);control.append(o);}}
            else {control=el('input');control.type=def.type==='boolean'?'checkbox':(['number','integer'].includes(def.type)?'number':'text');if(control.type==='number')control.step=def.type==='integer'?'1':'any';}
            label.append(control);part.append(label);fields.push({name,def,control});
          }
          const action=el('select');for(const name of ['','accept','decline','cancel']){const o=el('option',name||'Choose a response');o.value=name;action.append(o);}part.append(action);
          replies.set(id,()=>{if(!action.value)throw Error('Choose a response for each request.');if(action.value!=='accept')return {action:action.value};const content={};for(const {name,def,control} of fields){if(control.type==='checkbox'){content[name]=control.checked;continue;}if(control.value===''){if(schema.required?.includes(name))throw Error(`${name} is required`);continue;}if(Array.isArray(def.enum))content[name]=JSON.parse(control.value);else if(['integer','number'].includes(def.type)){const n=Number(control.value);if(!Number.isFinite(n)||(def.type==='integer'&&!Number.isSafeInteger(n)))throw Error(`${name} must be a valid ${def.type}`);content[name]=n;}else if(def.type==='array'){content[name]=JSON.parse(control.value);}else content[name]=control.value;}return {action:'accept',content};});
        }
        box.append(part);
      }
      if(replies.size||!Object.keys(item.input_requests).length){const consent=el('label');consent.className='consent';const check=el('input');check.type='checkbox';consent.append(check,document.createTextNode('Send only these reviewed answers to the requesting MCP server.'));box.append(consent,button('Submit reviewed response',async()=>{if(!check.checked)throw Error('Review and confirm these answers.');const responses=Object.fromEntries([...replies].map(([key,value])=>[key,value()]));await call({action:'respond_mcp',assignment,response:{interaction_id:item.interaction_id,request_key:crypto.randomUUID(),responses}});await waits();}));}
      $('mcp-waits').append(box);
    }
    if(result.truncated)$('mcp-waits').append(el('p','More requests exist. Resolve this page and refresh.'));
  }
  $('mcp-waits-refresh').onclick=()=>act(waits);
})();
