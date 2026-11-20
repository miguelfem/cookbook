'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

// Run the shipped handlers across the native-command boundary. Rendering and
// real browser/vault consent are covered separately by native qualification.
class Element {
  constructor(tag) { this.tagName=tag;this.children=[];this.listeners={};this.dataset={};this.textContent='';this._value=''; }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children=children; }
  remove() { this.removed=true; }
  addEventListener(event, handler) { this.listeners[event]=handler; }
  setAttribute(name,value) { this[name]=value; }
  closest() { return null; }
  focus() {}
  get firstChild() { return this.children[0]; }
  get lastChild() { return this.children.at(-1); }
  get options() { return this.children; }
  get selectedOptions() { return this.children.filter(c=>c.value===this.value); }
  get value() { return this._value|| (this.tagName==='select' ? this.firstChild?.value||'' : ''); }
  set value(value) { this._value=value; }
}
const descendants = e => [e,...e.children.flatMap(child=>child instanceof Element?descendants(child):[])];
const settle = async()=>{for(let i=0;i<8;i++)await new Promise(setImmediate);};
async function fixture(auth,discoveryReceipt,discoveryAccounts=[],approvalOptions={}){
  const nodes=new Map(),sent=[],native=[],cancelled=[];
  const node=id=>{if(!nodes.has(id))nodes.set(id,new Element('div'));return nodes.get(id);};
  const connection={label:'Synthetic service',endpoint:'https://service.example/mcp',auth,
    ...(auth.kind==='bearer'?{credential_label:'Project API key'}:{}),input_schema:{},output_schema:{},...approvalOptions.connection};
  for(const [id,value] of Object.entries({'global-client':'codex','mcp-auth':'browser','mcp-feature':'tools','mcp-label':'Synthetic service',transport:'http',protocol:'2026-07-28','mcp-key-header':'bearer'}))node(id).value=value;
  let approved=false;
  const invoke=async(command,args)=>{
    if(command==='desktop_status')return {assignment_id:'assignment'};
    if(command==='oauth_discover_begin'){
      assert.ok(discoveryReceipt);
      assert.equal(args.request.connection.auth.kind,'oauth');
      assert.equal(args.request.connection.auth.credential_ref,'pending');
      native.push(JSON.parse(JSON.stringify(args.request)));
      return {flow_id:'discovery-flow',state:'discovering',grant_created:false};
    }
    if(command==='oauth_cancel'){cancelled.push(args.flowId);return {};}
    if(command==='oauth_status'){
      assert.equal(args.flowId,'discovery-flow');
      if(discoveryReceipt.waiting)return {state:'waiting'};
      if(discoveryReceipt.stopped)return {state:'stopped',message:'Authorization cancelled'};
      return {state:'connected',receipt:discoveryReceipt};
    }
    assert.equal(command,'owner_request','Saved OAuth account must not start another browser flow');
    const r=JSON.parse(JSON.stringify(args.request));sent.push(r);
    if(r.action==='preview_agent'){
      if(approvalOptions.previewWait)await approvalOptions.previewWait;
      return {plan_id:'agent-plan',trust_apr_run:r.trust_apr_run,config_path:'/synthetic/config.toml',skill_path:'/synthetic/SKILL.md',server_entry:r.trust_apr_run?{tools:{apr_run:{approval_mode:'approve'}}}:{},changes:['Connect CookBook']};
    }
    if(r.action==='apply_agent')return {changed:true};
    if(r.action==='list_assignments')return {assignments:[{assignment_id:'assignment',state:'active',name:'Synthetic',root:'/synthetic'}]};
    if(r.action==='list_approvals')return approvalOptions.paginated?{requests:[],next_cursor:r.after?null:'later'}:{requests:approved?[]:[{name:'read',rationale:'Bounded read',connection,request_id:'request',digest:'digest',state:'pending'}]};
    if(r.action==='list_accounts'){
      if(approvalOptions.accounts)return {accounts:approvalOptions.accounts,next_cursor:null};
      if(discoveryReceipt&&r.connection.mcp)return {accounts:discoveryAccounts,next_cursor:null};
      assert.deepEqual(r.connection,connection);
      return r.after?{accounts:[{account_id:'account-2',revision:3,label:'Existing account',enabled:true,compatible:true}],next_cursor:null}:{accounts:[{account_id:'incompatible',revision:1,label:'Different scope',enabled:true,compatible:false}],next_cursor:'page-2'};
    }
    if(r.action==='discover_oauth')return {issuer:'https://auth.example',resource:r.resource,scopes:['read']};
    if(r.action==='connect_mcp_account'){
      if(discoveryReceipt.account&&!discoveryAccounts.some(a=>a.account_id===discoveryReceipt.account.account_id))discoveryAccounts.push({...discoveryReceipt.account,label:'Saved service',enabled:true,compatible:true});
      return discoveryReceipt;
    }
    if(r.action==='approve'){approved=true;return {};}
    if(r.action==='preview_account_reuse')return {review_digest:'review-digest',old_contract:{transport:{args:['--lock-sha256','old']}},new_contract:{transport:{args:['--lock-sha256','new']}},old_operation:{input_schema:{host:{const:'service.example'}}},new_operation:{input_schema:{host:{const:'service.example'}}}};
    throw Error('Unexpected action '+r.action);
  };
  const context={document:{getElementById:node,createElement:tag=>new Element(tag),createTextNode:text=>text,querySelectorAll:()=>[]},window:{__TAURI__:{core:{invoke},dialog:{confirm:async()=>true}},addEventListener(){},dispatchEvent(){},scrollTo(){}},location:{hash:''},Event:class {},crypto:{randomUUID:()=> 'synthetic'},setTimeout};
  if(discoveryReceipt?.waiting){let now=0;context.Date={now:()=>now+=310001};}
  vm.createContext(context);vm.runInContext(fs.readFileSync(path.join(__dirname,'../ui/setup.js'),'utf8'),context);
  await settle();
  return {node,sent,native,cancelled,elements:()=>descendants(node('requests'))};
}
test('Codex recipe trust is explicit, reviewed and invalidated when the choice changes',async()=>{
  const ui=await fixture({kind:'none'});
  await ui.node('preview-agent').listeners.click();await settle();
  assert.equal(ui.sent.find(r=>r.action==='preview_agent').trust_apr_run,false);
  ui.node('trust-apr-run').checked=true;ui.node('trust-apr-run').listeners.change();
  await ui.node('apply-agent').listeners.click();await settle();
  assert.ok(!ui.sent.some(r=>r.action==='apply_agent'));
  await ui.node('preview-agent').listeners.click();await settle();
  assert.equal(ui.sent.filter(r=>r.action==='preview_agent').at(-1).trust_apr_run,true);
  assert.match(ui.node('agent-settings').textContent,/approval_mode.*\n.*approve|"approval_mode": "approve"/);
  assert.match(ui.node('agent-result').textContent,/every project using this user-level CookBook connection/);
  await ui.node('apply-agent').listeners.click();await settle();
  assert.deepEqual(ui.sent.find(r=>r.action==='apply_agent'),{action:'apply_agent',plan_id:'agent-plan'});
  ui.node('global-client').value='claude';ui.node('global-client').listeners.change();
  assert.equal(ui.node('trust-apr-run').checked,false);
  assert.equal(ui.node('run-trust-choice').hidden,true);
});
test('late agent preview cannot enable apply after a client or trust choice changes',async()=>{
  let release;const previewWait=new Promise(resolve=>{release=resolve;});
  const ui=await fixture({kind:'none'},undefined,[],{previewWait});
  ui.node('preview-agent').listeners.click();await settle();
  assert.equal(ui.node('preview-agent').disabled,true);
  ui.node('trust-apr-run').checked=true;ui.node('trust-apr-run').listeners.change();
  await ui.node('preview-agent').listeners.click();await settle();
  assert.equal(ui.sent.filter(r=>r.action==='preview_agent').length,1,'Serialize previews so an old backend completion cannot replace a newer plan');
  release();await settle();
  assert.equal(ui.node('preview-agent').disabled,false);
  assert.equal(ui.node('apply-agent').disabled,true);
  await ui.node('apply-agent').listeners.click();await settle();
  assert.ok(!ui.sent.some(r=>r.action==='apply_agent'));
  await ui.node('preview-agent').listeners.click();await settle();
  assert.equal(ui.sent.filter(r=>r.action==='preview_agent').length,2);
  assert.equal(ui.node('apply-agent').disabled,false);
});
test('changed adapter account needs explicit digest-bound review and resets when selection changes',async()=>{
  const ui=await fixture({kind:'bearer',credential_ref:'pending'},undefined,[],{
    connection:{endpoint:'stdio:local',mcp:{transport:{kind:'stdio',credential_env:'SERVICE_KEY'}}},
    accounts:[{account_id:'old-account',revision:4,label:'Saved provider',enabled:true,compatible:false,review_required:true},{account_id:'other-provider',revision:1,label:'Other provider',enabled:true,compatible:false,review_required:false}],
  });
  const select=ui.elements().find(e=>e.tagName==='select'&&e.options.some(o=>o.value==='old-account'));
  assert.equal(select.value,'','An adapter update must never be auto-selected');
  assert.ok(!select.options.some(o=>o.value==='other-provider'));
  select.value='old-account';select.listeners.change();
  const [reviewConsent,ordinaryConsent]=ui.elements().filter(e=>e.tagName==='input'&&e.type==='checkbox');
  const approve=ui.elements().find(e=>e.textContent==='Approve access');
  const review=ui.elements().find(e=>e.textContent==='Review adapter update');
  ordinaryConsent.checked=true;await approve.listeners.click();await settle();
  assert.ok(!ui.sent.some(r=>r.action==='approve'));
  await review.listeners.click();await settle();
  assert.deepEqual(ui.sent.find(r=>r.action==='preview_account_reuse'),{action:'preview_account_reuse',request_id:'request',digest:'digest',account_id:'old-account',revision:4});
  assert.ok(ui.elements().some(e=>e.tagName==='pre'&&e.textContent.includes('old_contract')&&e.textContent.includes('new_operation')));
  await approve.listeners.click();await settle();
  assert.ok(!ui.sent.some(r=>r.action==='approve'),'Preview alone is not consent');
  reviewConsent.checked=true;select.value='';select.listeners.change();select.value='old-account';select.listeners.change();ordinaryConsent.checked=true;reviewConsent.checked=true;
  await approve.listeners.click();await settle();
  assert.ok(!ui.sent.some(r=>r.action==='approve'),'Changing the account invalidates its old review');
  await review.listeners.click();await settle();reviewConsent.checked=true;ordinaryConsent.checked=true;
  await approve.listeners.click();await settle();
  assert.deepEqual(ui.sent.find(r=>r.action==='approve'),{action:'approve',request_id:'request',digest:'digest',secret:null,options:{all_projects:false,saved_account:null,reviewed_account_reuse:{account_id:'old-account',revision:4,review_digest:'review-digest'}}});
  assert.equal(ui.native.length,0);
});
for(const auth of [{kind:'bearer',credential_ref:'pending'},{kind:'oauth',credential_ref:'pending',config:{issuer:'https://auth.example',scopes:['read']}}]){
  test(`${auth.kind} approval reuses a compatible account from later pages without sharing the tool`,async()=>{
    const ui=await fixture(auth);
    assert.equal(ui.node('status').textContent,'');
    const account=ui.elements().find(e=>e.tagName==='select'&&e.options.some(o=>o.value==='account-2'));
    assert.equal(account.value,'account-2');
    assert.ok(!account.options.some(o=>o.value==='incompatible'));
    const consent=ui.elements().find(e=>e.tagName==='input'&&e.type==='checkbox');
    const approve=ui.elements().find(e=>e.tagName==='button'&&e.textContent==='Approve access');
    await approve.listeners.click();await settle();
    assert.ok(!ui.sent.some(r=>r.action==='approve'),'Unchecked approval must do nothing');
    consent.checked=true;await approve.listeners.click();await settle();
    assert.deepEqual(ui.sent.find(r=>r.action==='approve'),{action:'approve',request_id:'request',digest:'digest',secret:null,options:{all_projects:false,saved_account:{account_id:'account-2',revision:3}}});
  });
}
test('native approval paging reaches later requests without deciding earlier ones',async()=>{
  const ui=await fixture({kind:'none'},undefined,[],{paginated:true});
  await ui.elements().find(e=>e.textContent==='Next requests').listeners.click();await settle();
  assert.equal(ui.sent.filter(r=>r.action==='list_approvals').at(-1).after,'later');
  assert.ok(ui.elements().some(e=>e.textContent==='First requests'));
  assert.ok(!ui.sent.some(r=>['approve','reject'].includes(r.action)));
});
test('saved OAuth account survives discovery failure and leaves a clear retry state',async()=>{
  const receipt={account:{account_id:'account-1',revision:1},status:'account_connected_discovery_failed',discovery:null,discovery_error:'MCP metadata unavailable',grant_created:false};
  const ui=await fixture({kind:'none'},receipt);
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-use-oauth').checked=true;
  ui.node('oauth-config').textContent=JSON.stringify({issuer:'https://auth.example',resource:'https://service.example/mcp',scopes:['read']});
  ui.node('mcp-form').listeners.submit({preventDefault(){}});
  await settle();
  assert.match(ui.node('status').textContent,/Account saved\. Tool discovery failed: MCP metadata unavailable.*Retry discovery/);
  assert.equal(ui.node('discovered-tools').children.length,0,'Cancel control is removed after completion');
  assert.equal(ui.node('connection-json').value,'','No operation is proposed from failed discovery');
});
test('stopped OAuth discovery removes the cancel control',async()=>{
  const ui=await fixture({kind:'none'},{stopped:true});
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-use-oauth').checked=true;
  ui.node('oauth-config').textContent=JSON.stringify({issuer:'https://auth.example',resource:'https://service.example/mcp',scopes:['read']});
  ui.node('mcp-form').listeners.submit({preventDefault(){}});
  await settle();
  assert.equal(ui.node('status').textContent,'Authorization cancelled');
  assert.equal(ui.node('discovered-tools').children.length,0);
});
test('metadata discovery reuses a durable saved account and allows a different login',async()=>{
  const account={account_id:'saved-after-restart',revision:4,label:'Existing login',enabled:true,compatible:true};
  const ui=await fixture({kind:'none'},{account,discovery:{tools:[]},grant_created:false},[account]);
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-use-oauth').checked=true;
  ui.node('oauth-config').textContent=JSON.stringify({issuer:'https://auth.example',resource:'https://service.example/mcp',scopes:['read']});
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.deepEqual(ui.native[0].saved_account,{account_id:'saved-after-restart',revision:4});
  assert.equal(ui.node('mcp-account-label').hidden,false);
  ui.node('mcp-account').value='';
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.equal(ui.native[1].saved_account,null,'An explicit different-account choice must open a new login');
});
test('metadata discovery does not choose between multiple compatible accounts',async()=>{
  const accounts=['first','second'].map(account_id=>({account_id,revision:1,label:account_id,enabled:true,compatible:true}));
  const ui=await fixture({kind:'none'},{discovery:{tools:[]}},accounts);
  ui.node('endpoint').value='https://service.example/mcp';ui.node('mcp-use-oauth').checked=true;
  ui.node('oauth-config').textContent=JSON.stringify({issuer:'https://auth.example',resource:'https://service.example/mcp',scopes:['read']});
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.equal(ui.native.length,0);
  assert.match(ui.node('status').textContent,/Choose the saved account/);
  ui.node('mcp-account').value='second';
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.deepEqual(ui.native[0].saved_account,{account_id:'second',revision:1});
});

test('browser sign-in discovers its configuration on the connection page',async()=>{
  const ui=await fixture({kind:'none'},{account:{account_id:'new-login',revision:1},discovery:{tools:[]}});
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.deepEqual(ui.sent.find(r=>r.action==='discover_oauth'),{action:'discover_oauth',resource:'https://service.example/mcp',metadata_url:null,client_id:'',scopes:[]});
  assert.equal(ui.native[0].connection.auth.config.resource,'https://service.example/mcp');
  assert.equal(ui.node('mcp-connect').disabled,false);
  assert.match(ui.node('status').textContent,/Account saved/);
});
for(const format of ['bearer','x-api-key','api-key']){
  test(`${format} key is saved once, then reused after discovery failure`,async()=>{
    const receipt={account:{account_id:'saved-key',revision:1},discovery:null,discovery_error:'MCP metadata unavailable',grant_created:false};
    const ui=await fixture({kind:'none'},receipt);
    ui.node('mcp-auth').value='key';ui.node('mcp-key-header').value=format;
    ui.node('endpoint').value='https://service.example/mcp';ui.node('mcp-secret').value='synthetic-key';
    ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
    const first=ui.sent.find(r=>r.action==='connect_mcp_account');
    assert.equal(first.secret,'synthetic-key');assert.equal(first.saved_account,null);
    assert.deepEqual(first.connection.auth,format==='bearer'?{kind:'bearer',credential_ref:'pending'}:{kind:'header',name:format,credential_ref:'pending'});
    assert.equal(ui.node('mcp-secret').value,'');
    assert.match(ui.node('status').textContent,/Account saved\. Tool discovery failed/);
    assert.ok(!ui.node('status').textContent.includes('synthetic-key'));
    ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
    const second=ui.sent.filter(r=>r.action==='connect_mcp_account')[1];
    assert.equal(second.secret,null);assert.deepEqual(second.saved_account,{account_id:'saved-key',revision:1});
    assert.equal(ui.native.length,0,'API keys must not trigger browser login');
    assert.equal(ui.node('connection-json').value,'','Failed discovery must not propose a tool');
  });
}
test('connection form suppresses double submission while browser sign-in starts',async()=>{
  const ui=await fixture({kind:'none'},{discovery:{tools:[]}});
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-form').listeners.submit({preventDefault(){}});
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.equal(ui.native.length,1);
});

test('polling timeout cancels native authorization before allowing retry',async()=>{
  const ui=await fixture({kind:'none'},{waiting:true});
  ui.node('endpoint').value='https://service.example/mcp';
  ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
  assert.deepEqual(ui.cancelled,['discovery-flow']);
  assert.match(ui.node('status').textContent,/Authorization expired/);
  assert.equal(ui.node('mcp-connect').disabled,false);
  assert.equal(ui.node('discovered-tools').children.length,0);
});

for(const mode of ['browser','key']){
  test(`${mode} automatic discovery pins the resolved protocol in the tool proposal`,async()=>{
    const receipt={account:{account_id:'saved',revision:1},discovery:{protocol:'2025-06-18',protocol_detected:true,tools:[{name:'read_summary',input_schema:{type:'object'}}]},grant_created:false};
    const ui=await fixture({kind:'none'},receipt);
    ui.node('endpoint').value='https://service.example/mcp';
    ui.node('protocol').value='auto';ui.node('mcp-auth').value=mode;
    if(mode==='key')ui.node('mcp-secret').value='synthetic-key';
    ui.node('mcp-form').listeners.submit({preventDefault(){}});await settle();
    const request=mode==='browser'?ui.native[0]:ui.sent.find(r=>r.action==='connect_mcp_account');
    assert.equal(request.automatic_protocol,true);
    assert.equal(request.connection.mcp.protocol,'2026-07-28');
    const select=descendants(ui.node('discovered-tools')).find(e=>e.tagName==='button'&&e.textContent==='Select for recipe connection');
    assert.ok(select);await select.listeners.click();await settle();
    const proposal=JSON.parse(ui.node('connection-json').value);
    assert.equal(proposal.mcp.protocol,'2025-06-18');
    assert.equal(proposal.mcp.tool.name,'read_summary');
    assert.equal(proposal.auth.credential_ref,'pending');
    assert.ok(!ui.sent.some(r=>r.action==='approve'));
    assert.ok(!ui.node('connection-json').value.includes('synthetic-key'));
  });
}
