'use strict';
// This host lives in a separate custom-protocol window with NO native/runtime
// capabilities. It never forwards an App's request to Tauri or another service.
(() => {
 const value=JSON.parse(document.getElementById('payload').textContent);
 const status=document.getElementById('status');
 const frame=document.createElement('iframe');frame.title='Read-only MCP App';frame.width='100%';frame.height='550';
 frame.setAttribute('sandbox','allow-scripts');frame.referrerPolicy='no-referrer';
 frame.setAttribute('allow',"camera 'none'; microphone 'none'; geolocation 'none'; clipboard-read 'none'; clipboard-write 'none'");
 let initialized=false,ready=false,start=performance.now(),messages=0;
 const destroy=()=>{frame.remove();window.removeEventListener('message',listen);};
 const listen=event=>{
  if(event.source!==frame.contentWindow||event.origin!=='null')return;
  if(performance.now()-start>1000){start=performance.now();messages=0;}
  if(++messages>100){destroy();status.textContent='App message limit exceeded.';return;}
  const data=event.data;try{if(!data||JSON.stringify(data).length>8192)return;}catch{return;}
  if(data.type==='cookbook:document-loaded'){
   if(!ready)status.textContent='Read-only App document loaded · App protocol not initialized';
   return;
  }
  if(data.jsonrpc!=='2.0')return;
  const id=data.id,validId=typeof id==='string'?id.length<=128:Number.isSafeInteger(id);
  const reply=(result,error)=>{if(validId)frame.contentWindow?.postMessage({jsonrpc:'2.0',id,...(error?{error}:{result})},'*');};
  if(data.method==='ui/initialize'){
   if(initialized){reply(null,{code:-32600,message:'Already initialized'});return;}
   if(data.params?.protocolVersion!=='2026-01-26'){reply(null,{code:-32602,message:'Unsupported Apps protocol'});return;}
   initialized=true;reply({protocolVersion:'2026-01-26',hostInfo:{name:'CookBook APR',version:'0.2.0'},hostCapabilities:{},hostContext:{theme:'light',displayMode:'inline',availableDisplayModes:['inline'],locale:navigator.language}});
  }else if(data.method==='ui/notifications/initialized'&&initialized){
   ready=true;
   frame.contentWindow?.postMessage({jsonrpc:'2.0',method:'ui/notifications/tool-result',params:value.tool_result},'*');
   status.textContent='Isolated read-only preview · no host tool access';
  }else if(data.method==='ui/notifications/size-changed'&&initialized){
   const height=data.params?.height;if(Number.isFinite(height))frame.height=String(Math.max(180,Math.min(1000,height)));
  }else if(data.method&&validId)reply(null,{code:-32601,message:'Not available in CookBook read-only Apps'});
 };
 window.addEventListener('message',listen);window.addEventListener('pagehide',destroy);
 frame.src=value.document_url;document.querySelector('main').append(frame);
})();
