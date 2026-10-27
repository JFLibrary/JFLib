// One browser adapter for every jfguiLib app. All application UI is rendered in Rust.
import {openFilesStorage} from "./storage.js";
import {openSecrets, encodeSecrets} from "./secrets.js";
import * as handles from "./handles.js";
import {Shadow, decode as decodeA11y} from "./a11y.js";
const canvas=document.querySelector("#screen"), input=document.querySelector("#input"), status=document.querySelector("#status");
const fileInput=document.querySelector("#files"), controlButton=document.querySelector("#control");
const viewport=document.querySelector("#viewport"), a11yRoot=document.querySelector("#a11y");
const app=document.body.dataset.app, params=new URLSearchParams(location.search), worker=new Worker("worker.js",{type:"module"});
const ctx=canvas.getContext("2d",{alpha:false}), text=new TextDecoder();
let started=false, enabled=params.get("agent")==="1", nextId=1, composing=false, openRequest=0, database=null, pasteModifiers=2;
let bridgeDisconnect=null, bridgeAttempt=0, secretStore=null, handleStore=null, unsaved=false;
window.addEventListener("beforeunload",e=>{ if(unsaved) {e.preventDefault();e.returnValue="";} });
// sound (audio.js, lib/plat/web_audio.rs): loaded with the first stream
let audio=null;
const audioHost=()=>audio||=import("./audio.js").then(x=>new x.AudioHost(worker));
const pending=new Map(), ports=new Set();
// One acknowledged batch at a time: a busy worker must not accumulate old frames
// or hover positions. Input edges and every pressed-pointer sample stay ordered.
const outgoing=[];
let inFlight=false, pumpFrame=0, pumpTimer=0;
function schedule() {
  if(!started || inFlight || pumpFrame || pumpTimer || !outgoing.length) return;
  if(document.hidden) pumpTimer=setTimeout(flush,33);
  else pumpFrame=requestAnimationFrame(flush);
}
function flush() {
  pumpFrame=0; pumpTimer=0;
  if(!started || inFlight || !outgoing.length) return;
  inFlight=true;
  worker.postMessage({type:"batch",messages:outgoing.splice(0,128)});
}
function send(message) {
  if(message.type==="start") {worker.postMessage(message);return;}
  const last=outgoing[outgoing.length-1];
  if(message.hover && last?.hover && last.event[2]===message.event[2]) outgoing[outgoing.length-1]=message;
  else outgoing.push(message);
  schedule();
}
document.addEventListener("visibilitychange",()=>{
  if(pumpFrame) cancelAnimationFrame(pumpFrame);
  if(pumpTimer) clearTimeout(pumpTimer);
  pumpFrame=0; pumpTimer=0; schedule();
});
const complete=(id,status)=>id && send({type:"complete",id,status});
const errorCode=error=>error?.name==="QuotaExceededError" ? -2 : error?.name==="NotAllowedError" || error?.name==="SecurityError" ? -1 : -5;
function sizes() { return {width:Math.max(1,Math.round(canvas.clientWidth*devicePixelRatio)), height:Math.max(1,Math.round(canvas.clientHeight*devicePixelRatio)), dpi:Math.round(96*devicePixelRatio)}; }
function report(message) { status.textContent=message; }
async function storage() {
  return openFilesStorage();
}
async function persist(path, bytes) {
  if(!database) throw new DOMException("Browserspeicher nicht verfügbar","NotAllowedError");
  await new Promise((resolve,reject)=>{
    const t=database.transaction("files","readwrite");
    t.objectStore("files").put({key:app+":"+path,app,path,bytes});
    t.oncomplete=resolve; t.onabort=t.onerror=()=>reject(t.error || new Error("Speichern abgebrochen"));
  });
}
function controlRequest(message) {
  if(!enabled || !started) return Promise.reject(new Error("Agentensteuerung ist deaktiviert oder startet noch."));
  if(pending.size>=64) return Promise.reject(new Error("Zu viele gleichzeitige Anfragen."));
  const id=nextId++;
  return new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>{ pending.delete(id); reject(new Error("Agentenanfrage: Zeitüberschreitung")); },30000);
    pending.set(id,{resolve,reject,timer}); send({...message,id});
  });
}
const rpc=(request,{session="page"}={})=>controlRequest({type:"rpc",request,session});
const closeSession=session=>controlRequest({type:"closeSession",session});
function activate(on) {
  enabled=on; controlButton.setAttribute("aria-pressed",String(on));
  controlButton.textContent=on ? "Agentensteuerung abschalten" : "Agentensteuerung aktivieren";
  if(started) send({type:"agent",enabled:on});
  if(on) {
    window.jfguiAgent=Object.freeze({
      request:rpc,
      closeSession,
      connect(port) {
        if(!(port instanceof MessagePort) || !enabled) throw new Error("Eine explizite MessagePort-Verbindung ist erforderlich.");
        const session="port-"+nextId++;
        ports.add(port); port.onmessage=async({data})=>{
          try { port.postMessage(await rpc(data,{session})); }
          catch(error) { port.postMessage({jsonrpc:"2.0",id:data?.id??null,error:{code:-32000,message:error.message}}); }
        }; port.start(); report("Agent verbunden");
        return ()=>{port.close();ports.delete(port);if(enabled&&started)closeSession(session).catch(()=>{});};
      },
      disconnect:()=>activate(false),
      async connectBridge(options) {
        if(!enabled || !started) throw new Error("Agentensteuerung ist deaktiviert oder startet noch.");
        const attempt=++bridgeAttempt;
        if(bridgeDisconnect) {const stop=bridgeDisconnect;bridgeDisconnect=null;await stop();}
        const {connectBridge}=await import("./bridge.js");
        if(!enabled || attempt!==bridgeAttempt) throw new Error("Agentenverbindung wurde widerrufen.");
        const stop=await connectBridge(window.jfguiAgent,options,message=>{if(enabled)report(message);});
        if(!enabled || attempt!==bridgeAttempt) {await stop();throw new Error("Agentenverbindung wurde widerrufen.");}
        bridgeDisconnect=stop;
        return async()=>{if(bridgeDisconnect===stop)bridgeDisconnect=null;await stop();};
      },
    }); report("Agentensteuerung aktiv");
  } else {
    ++bridgeAttempt;
    if(bridgeDisconnect) {const stop=bridgeDisconnect;bridgeDisconnect=null;stop().catch(()=>{});}
    delete window.jfguiAgent; for(const port of ports) port.close(); ports.clear();
    for(const p of pending.values()) { clearTimeout(p.timer); p.reject(new Error("Agentensteuerung abgeschaltet")); } pending.clear();
    report("Agentensteuerung aus");
  }
}
controlButton.onclick=()=>activate(!enabled);
function event(type,button=0,x=0,y=0,data=0,mods=0,hover=false) { if(started) send({type:"event",event:[type,button,mods,0,Math.round(x),Math.round(y),data>>>0],hover}); }
const mods=e=>(e.shiftKey?1:0)|(e.ctrlKey||e.metaKey?2:0)|(e.altKey?4:0)|(e.getModifierState?.("AltGraph")?16:0);
const pos=e=>{const r=canvas.getBoundingClientRect();return [(e.clientX-r.left)*devicePixelRatio,(e.clientY-r.top)*devicePixelRatio];};
const button=e=>[1,3,2,4,5][e.button]||1;
const pen=e=>{
  const type=e.pointerType==="pen"?(e.button===5?2:1):e.pointerType==="touch"?3:0;
  if(!type)return 0;
  const pressure=Math.max(1,Math.min(65535,Math.round((e.pressure??1)*65535)));
  const tilt=value=>Math.max(-63,Math.min(63,Math.round(value||0)))&127;
  return pressure|(type<<16)|(tilt(e.tiltX)<<18)|(tilt(e.tiltY)<<25);
};
// The accessibility tree as an ARIA shadow tree over the canvas (a11y.js; ?a11y=0: none).
const shadow=params.get("a11y")!=="0" && a11yRoot ? new Shadow(a11yRoot,{input,act:m=>send({type:"a11y-act",...m})}) : null;
if(shadow) Object.defineProperty(window,"jfguiA11y",{value:shadow}); // read-only handle for tests
canvas.onpointerdown=e=>{if(!e.isPrimary)return;e.preventDefault();if(shadow)shadow.follow=false;input.focus({preventScroll:true});canvas.setPointerCapture(e.pointerId);event(2,button(e),...pos(e),pen(e),mods(e));};
canvas.onpointermove=e=>{
  if(!e.isPrimary)return;
  const samples=e.buttons ? e.getCoalescedEvents?.() : null;
  for(const sample of samples?.length ? samples : [e]) event(1,0,...pos(sample),pen(sample),mods(e),!e.buttons);
};
canvas.onpointerup=e=>{if(!e.isPrimary)return;event(3,button(e),...pos(e),pen(e),mods(e));if(canvas.hasPointerCapture(e.pointerId))canvas.releasePointerCapture(e.pointerId);};
canvas.onpointercancel=e=>event(3,1,...pos(e),0,mods(e));
canvas.onpointerleave=()=>event(12);
canvas.oncontextmenu=e=>e.preventDefault();
canvas.onwheel=e=>{e.preventDefault();const factor=e.deltaMode===1?30:e.deltaMode===2?canvas.clientHeight:1;const dy=Math.max(-32768,Math.min(32767,Math.round(-e.deltaY*factor))),dx=Math.max(-32768,Math.min(32767,Math.round(e.deltaX*factor)));event(4,0,...pos(e),(dy&65535)|((dx&65535)<<16),mods(e));};
const keys={Backspace:8,Tab:9,Enter:13,Shift:16,Control:17,Alt:18,Pause:19,CapsLock:20,Escape:27,' ':32,PageUp:33,PageDown:34,End:35,Home:36,ArrowLeft:37,ArrowUp:38,ArrowRight:39,ArrowDown:40,Insert:45,Delete:46,Meta:91};
function key(e,down) {
  if(e.isComposing || composing) return;
  // Ordinary paste belongs to the DOM clipboard event; Alt-modified app shortcuts do not.
  if((e.ctrlKey||e.metaKey) && !e.altKey && e.key.toLowerCase()==="v") {pasteModifiers=down?2|(e.shiftKey?1:0):2;return;}
  const k=keys[e.key] || (/^F\d+$/.test(e.key)?111+Number(e.key.slice(1)):e.key.length===1?e.key.toUpperCase().charCodeAt(0):0);
  if(k) event(down?5:6,0,0,0,k | (e.repeat?0x80000000:0),mods(e));
  // Printable characters, Enter and paste use beforeinput, avoiding duplicated text.
  const clipboardKey=["v","c","x"].includes(e.key.toLowerCase()) && !e.altKey && (!e.shiftKey || e.key.toLowerCase()==="v");
  if(e.key==="Tab" || e.key.startsWith("Arrow") || ["Backspace","Delete","Escape"].includes(e.key) || ((e.ctrlKey||e.metaKey)&&!clipboardKey)) e.preventDefault();
}
// The keyboard reaches the app through the hidden #input and, while the DOM focus follows the
// app's focus, through the elements of the shadow tree: the same handlers for both.
const editable=el=>el===input || el.tagName==="TEXTAREA" || (el.tagName==="INPUT" && el.type!=="range");
function keydown(e) {
  if(shadow) shadow.follow=true;
  if(e.target!==input) shadow?.claim(e.target);
  key(e,true);
  if(e.target===input || e.isComposing || composing) return;
  // shadow elements: the app moves carets, values and pages, not the browser
  const field=editable(e.target);
  if(["PageUp","PageDown","Home","End"].includes(e.key) || (!field && e.key===" ")) e.preventDefault();
  if(!field && e.key.length===1 && !e.ctrlKey && !e.metaKey && !e.altKey) send({type:"text",text:e.key});
}
for(const target of [input,a11yRoot].filter(Boolean)) {
  target.addEventListener("keydown",keydown); target.addEventListener("keyup",e=>key(e,false));
  target.addEventListener("compositionstart",()=>{composing=true;});
  target.addEventListener("compositionend",e=>{composing=false;if(e.data)send({type:"text",text:e.data});if(e.target===input)input.value="";else shadow?.restore(e.target);});
  target.addEventListener("beforeinput",e=>{
    if(e.target!==input) shadow?.claim(e.target);
    if(e.isComposing || composing) return;
    if(e.inputType==="insertText" && e.data) {e.preventDefault();send({type:"text",text:e.data});}
    else if(e.inputType==="insertLineBreak" || e.inputType==="insertParagraph") e.preventDefault();
  });
  // #input stays empty; a shadow field or range edited otherwise tells the app (a11y.js)
  target.addEventListener("input",e=>{if(composing)return;if(e.target===input)input.value="";else shadow?.edited(e.target,e.inputType);});
  target.addEventListener("paste",e=>{
    e.preventDefault(); const value=e.clipboardData?.getData("text/plain")||"";
    send({type:"clipboard",text:value,html:e.clipboardData?.getData("text/html")||"",rtf:e.clipboardData?.getData("text/rtf")||e.clipboardData?.getData("application/rtf")||""});
    // The browser clipboard event arrives after keydown; dispatch paste after importing it.
    event(5,0,0,0,86,pasteModifiers);event(6,0,0,0,86,pasteModifiers);pasteModifiers=2;
  });
}
// The app has the keyboard while the DOM focus is anywhere in the viewport.
viewport.addEventListener("focusin",e=>{if(!viewport.contains(e.relatedTarget))event(10);});
viewport.addEventListener("focusout",e=>{if(!viewport.contains(e.relatedTarget))event(11);});
let lastSize="";
function resize() {if(started){const s=sizes(),key=`${s.width},${s.height},${s.dpi}`;if(key!==lastSize){lastSize=key;event(9,0,0,0,s.dpi);event(8,0,s.width,s.height);shadow?.resized();screens();}}}
// the monitors (lib/plat/web_screen.rs): window.screen in CSS pixels, or every screen where the
// page may already ask (getScreenDetails with "window-management" granted; never a prompt);
// one line each: x y w h, the work area x y w h, dpi, refresh (mHz, unknown: 0), primary, name
let lastScreens="", screenDetails=null;
function screenLine(s,primary) {
  const n=v=>Math.round(Number(v)||0);
  return [n(s.left),n(s.top),n(s.width),n(s.height),n(s.availLeft),n(s.availTop),n(s.availWidth||s.width),n(s.availHeight||s.height),
    Math.round(96*(s.devicePixelRatio||devicePixelRatio)),0,primary?1:0,String(s.label||"").replace(/[\r\n]/g," ")].join(" ");
}
async function screens() {
  if(!started) return;
  let text=screenLine(screen,true);
  try {
    if(!screenDetails && window.getScreenDetails && (await navigator.permissions.query({name:"window-management"})).state==="granted") {
      screenDetails=await getScreenDetails();
      screenDetails.addEventListener("screenschange",screens);
    }
    if(screenDetails?.screens?.length) text=screenDetails.screens.map(s=>screenLine(s,s.isPrimary)).join("\n");
  } catch {}
  if(text!==lastScreens) {lastScreens=text;send({type:"screens",text});}
}
screen.addEventListener?.("change",screens);
new ResizeObserver(resize).observe(canvas);
window.addEventListener("resize",resize);
async function importFiles(files) {
  for(const file of files) {
    const path="/imports/"+file.name.replace(/[\\/\0]/g,"_");
    send({type:"import",path,bytes:await file.arrayBuffer(),dropped:true,durable:false});
  }
}
// The user's own files (handles.js): the picker's handle bound to a /handles/ path, an opened
// file imported there (its bytes count as stored: they are the file's).
async function pickHandles(kind, {name="", start="", exts=[]}) {
  const r=start && handleStore.byPath.get(start);
  const options={types:handles.types(exts),...(r?{startIn:r.handle}:{}),id:("jfgui-"+app).replace(/[^A-Za-z0-9_-]/g,"_").slice(0,32)};
  if(kind==="save") return [await showSaveFilePicker({...options,suggestedName:name || undefined})];
  return showOpenFilePicker({...options,multiple:kind==="many"});
}
async function importHandle(handle, path, dropped) {
  send({type:"import",path,bytes:await handles.readHandle(handle,true),dropped,durable:true});
}
// file_dialog's payload: u32 flags, name, start, u32 count, extensions (strings: u32 length + UTF-8)
function dialogPayload(data) {
  const v=new DataView(data.buffer,data.byteOffset,data.byteLength);let at=0;
  const u32=()=>{const x=v.getUint32(at,true);at+=4;return x;};
  const str=()=>{const n=u32();const t=text.decode(data.subarray(at,at+n));at+=n;return t;};
  const flags=u32(),name=str(),start=str(),exts=[];
  for(let n=u32();n>0;n--) exts.push(str());
  return {flags,name,start,exts};
}
document.querySelector("#open").onclick=()=>{fileInput.value="";fileInput.click();};
fileInput.onchange=async()=>{try{await importFiles(fileInput.files);complete(openRequest,2);}catch(e){complete(openRequest,errorCode(e));report(e.message);}openRequest=0;};
fileInput.oncancel=()=>{complete(openRequest,-4);openRequest=0;};
canvas.ondragover=e=>e.preventDefault();canvas.ondrop=e=>{e.preventDefault();importFiles(e.dataTransfer.files).catch(e=>report(e.message));};
async function service(m) {
  const data=new Uint8Array(m.data), value=()=>text.decode(data);
  try {
    if(m.kind===1 || m.kind===2) {
      const len=new DataView(m.data).getUint32(0,true);
      if(len>data.length-4) throw new Error("Ungültiger Dateiauftrag");
      const path=text.decode(data.subarray(4,4+len)), bytes=data.slice(4+len).buffer;
      // a file of the user's own: written back into it
      if(m.kind===1 && path.startsWith("/handles/")) {
        if(!handleStore) {complete(m.id,-3);return;}
        await handles.writeBack(handleStore,path,bytes);complete(m.id,0);report("In die Datei geschrieben: "+path.split("/").pop());
      }
      else if(m.kind===1) {await persist(path,bytes);complete(m.id,0);report("Im Browserspeicher gesichert");}
      else {const url=URL.createObjectURL(new Blob([bytes]));const link=document.createElement("a");link.href=url;link.download=path.split("/").pop()||"document";link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);complete(m.id,1);report("Download angefordert");}
    } else if(m.kind===3 && handleStore) {
      // pick_file with handles: the files imported as drops, each bound to its file
      const accept=value().split(/[,;\s]+/).filter(Boolean);
      let chosen;
      try {chosen=await pickHandles("many",{exts:accept});} catch(error) {complete(m.id,error?.name==="AbortError"?-4:errorCode(error));return;}
      for(const h of chosen) await importHandle(h,await handles.bind(handleStore,h),true);
      complete(m.id,2);
    }
    else if(m.kind===3) {openRequest=m.id;fileInput.accept=value();fileInput.value="";fileInput.click();}
    // file_dialog: the system's open / save picker; the answer is the bound path
    else if(m.kind===12) {
      if(!handleStore) {complete(m.id,-3);return;}
      const d=dialogPayload(data), save=(d.flags&1)!==0;
      let handle;
      try {[handle]=await pickHandles(save?"save":"one",d);} catch(error) {send({type:"file",id:m.id,status:error?.name==="AbortError"?-4:errorCode(error)});return;}
      const path=await handles.bind(handleStore,handle);
      if(!save) await importHandle(handle,path,false);
      send({type:"file",id:m.id,status:2,path});
    }
    // reopen: a remembered file read again (the permission asked for from the user's gesture)
    else if(m.kind===20) {
      const r=handleStore?.byPath.get(value());
      if(!r) {complete(m.id,-3);return;}
      await importHandle(r.handle,r.path,false);complete(m.id,2);
    }
    else if(m.kind===4) {if(!navigator.clipboard?.readText){complete(m.id,-3);return;}send({type:"clipboard",text:await navigator.clipboard.readText()});complete(m.id,2);}
    else if(m.kind===5) {if(!navigator.clipboard?.writeText){complete(m.id,-3);return;}await navigator.clipboard.writeText(value());complete(m.id,2);}
    else if(m.kind===6) {const v=new DataView(m.data);input.style.left=Math.max(0,Math.min(canvas.clientWidth-2,v.getInt32(0,true)/devicePixelRatio))+"px";input.style.top=Math.max(0,Math.min(canvas.clientHeight-20,v.getInt32(4,true)/devicePixelRatio))+"px";input.style.height=Math.max(16,v.getInt32(8,true)/devicePixelRatio)+"px";}
    else if(m.kind===7) canvas.style.cursor=["default","text","pointer","crosshair","move","wait","ns-resize","ew-resize","nwse-resize","nesw-resize","none"][new DataView(m.data).getUint32(0,true)]||"default";
    else if(m.kind===8) document.title=value();
    // unsaved work (plat::set_unsaved): leaving the page asks first
    else if(m.kind===22) unsaved=data[0]===1;
    else if(m.kind===9) {const url=new URL(value());if(!["https:","http:","mailto:"].includes(url.protocol))throw new Error("URL nicht unterstützt");if(!window.open(url.href,"_blank","noopener,noreferrer"))complete(m.id,-1);else complete(m.id,2);}
    else if(m.kind===10) {const on=data[0]===1;if(on!==enabled)activate(on);}
    // a change of the secret store (secrets.js): encrypted and committed in order
    else if(m.kind===13) {
      if(!secretStore) {data.fill(0);complete(m.id,-3);return;}
      secretStore.change(data).then(()=>complete(m.id,2),error=>{complete(m.id,errorCode(error));report("Geheimnis nicht gespeichert: "+error.message);});
    }
    else if(m.kind===11) {
      // the differences of the accessibility tree; a mirror that went wrong asks for all anew
      if(shadow) try {shadow.apply(decodeA11y(m.data));} catch(error) {console.error(error);send({type:"a11y",on:true});}
    }
    else if(m.kind===14) audioHost().then(h=>h.request(m.data)).catch(error=>console.warn("Audio:",error));
    // the colour pick (lib/plat/web_screen.rs): the EyeDropper, opened from the user's gesture
    else if(m.kind===21) {
      if(!window.EyeDropper) {send({type:"color",id:m.id,status:-3,rgb:0});return;}
      try {const r=await new EyeDropper().open();send({type:"color",id:m.id,status:2,rgb:parseInt(String(r.sRGBHex).slice(1),16)&0xFFFFFF});}
      catch(error) {send({type:"color",id:m.id,status:error?.name==="AbortError"?-4:errorCode(error),rgb:0});}
    }
    else complete(m.id,-3);
  } catch(error) {complete(m.id,errorCode(error));report(error.message);}
}
worker.onmessage=({data:m})=>{
  if(m.type==="frame") {
    // Assigning even an unchanged dimension clears the backing store and context.
    if(canvas.width!==m.width) canvas.width=m.width;
    if(canvas.height!==m.height) canvas.height=m.height;
    // the patches lie in one buffer (m.buffer, at their offsets), which goes back to the pool
    const patches=m.patches || [{x:0,y:0,width:m.width,height:m.height,pixels:m.pixels}];
    for(const p of patches) {
      const data=m.buffer ? new Uint8ClampedArray(m.buffer,p.offset,p.width*p.height*4) : new Uint8ClampedArray(p.pixels);
      ctx.putImageData(new ImageData(data,p.width,p.height),p.x,p.y);
    }
    const buffers=m.buffer ? [m.buffer] : patches.map(p=>p.pixels);
    worker.postMessage({type:"recycle",buffers},buffers);
  }
  else if(m.type==="batch-complete") {inFlight=false;schedule();}
  else if(m.type==="ready") {window.__jfguiCompute=m.compute;document.body.dataset.compute=m.compute?.mode || "scalar";document.body.dataset.computeWorkers=String(m.compute?.workers || 0);started=true;activate(enabled);screens();if(shadow)send({type:"a11y",on:true});input.focus({preventScroll:true});window.dispatchEvent(new Event("jfgui-ready"));}
  else if(m.type==="service") service(m);
  else if(m.type==="rpc" || (m.type==="error" && m.id)) {const p=pending.get(m.id);if(p){clearTimeout(p.timer);pending.delete(m.id);m.type==="error"?p.reject(new Error(m.message)):p.resolve(m.result);}}
  else if(m.type==="error") {report(m.message);console.error(m.message);}
  else if(m.type==="log") { (m.stream===2?console.error:console.log)(m.text); }
  else if(m.type==="exit") report(`Anwendung beendet (${m.code})`);
};
worker.onerror=e=>report(e.message);
// Idle ticks drive caret blink, animations and cooperative jobs without building a queue.
setInterval(()=>{if(started && !inFlight && !outgoing.length)send({type:"tick"});},33);
let initialFiles=[];
try {
  const restored=await storage();database=restored.database;initialFiles=restored.entries.filter(f=>f.app===app);
  if(restored.error)report("Gespeicherte Dateien sind lesbar; dauerhafte Datenübernahme derzeit nicht verfügbar.");
}catch(error){report("Browserspeicher nicht verfügbar; Dateien bleiben temporär.");}
// the user's files remembered from earlier visits: read again where the browser still lets the
// page read them (at most 64 MiB in all; the others through reopen, from a gesture)
if(handles.supported()) try {
  handleStore=await handles.openHandles(app);
  let budget=64<<20;
  for(const r of [...handleStore.records].sort((a,b)=>b.at-a.at)) {
    try {
      if(!await handles.permitted(r.handle,"read",false)) continue;
      const file=await r.handle.getFile();
      if(file.size>budget) continue;
      budget-=file.size;
      initialFiles.push({path:r.path,bytes:await file.arrayBuffer()});
    } catch {}
  }
}catch(error){handleStore=null;console.warn("Dateizugriff (File System Access) nicht verfügbar:",error);}
// the secret store: the app's values decrypted for the worker (none: plat::secret is Unavailable)
let initialSecrets=null;
try {
  const store=await openSecrets(app);
  if(store) {secretStore=store;initialSecrets=encodeSecrets(store.entries);for(const e of store.entries)e.value?.fill(0);store.entries=null;}
}catch(error){console.warn("Geheimnisspeicher nicht verfügbar:",error);}
const args=[];if(enabled)args.push("--agent");if(params.get("theme"))args.push("--theme",params.get("theme"));
lastScreens=screenLine(screen,true); // the first frame knows the monitor already
send({type:"start",url:`${app}.wasm`,mode:params.get("compute") || "auto",workers:Number(params.get("workers")) || undefined,...sizes(),args,files:initialFiles,screens:lastScreens,secrets:initialSecrets,capabilities:(database?4:0)|(navigator.clipboard?8:0)|(handleStore?65536:0)|(window.EyeDropper?131072:0)});
initialSecrets?.fill(0); // postMessage copied them
