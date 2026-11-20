'use strict';
// The ordinary recipe window remains script-src self. App scripts only run in
// a distinct no-capability preview window created by trusted native code.
(() => {
 const host=document.createElement('section');host.id='mcp-app-preview';
 const title=document.createElement('h2');title.textContent='MCP App preview';
 const note=document.createElement('p');note.textContent='Self-contained read-only Apps from approved completed recipe steps open in a separate window without runtime permissions. External network, tool calls, downloads and device access are disabled.';
 const form=document.createElement('form'),fields={};
 for(const[key,text]of [['run_id','Run ID'],['resource_step_id','UI resource step ID'],['tool_step_id','Related tool step ID']]){
  const label=document.createElement('label');label.textContent=text;const input=document.createElement('input');input.required=true;input.maxLength=256;label.append(input);form.append(label);fields[key]=input;
 }
 const open=document.createElement('button');open.textContent='Review and open App';form.append(open);
 const status=document.createElement('p');status.setAttribute('role','status');host.append(title,note,form,status);(document.getElementById('app-preview-host') || document.querySelector('main')).append(host);
 form.addEventListener('submit',async event=>{
  event.preventDefault();
  try{
   if(!await window.aprConfirm('Share this selected tool result with its approved App in a separate read-only window? No new tool calls or network access are allowed.'))return;
   const args=Object.fromEntries(Object.entries(fields).map(([key,input])=>[key,input.value.trim()]));
   await window.__TAURI__.core.invoke('app_preview_open',{args,consent:true});
   status.textContent='Opened in a separate read-only window. Close that window to end the preview.';
  }catch(error){status.textContent=error?.message||String(error);}
 });
})();
