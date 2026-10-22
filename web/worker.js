// Shared WASM worker. It never interprets app widgets or document semantics.
let wasm, memory, ready = false;
let module, failure, threaded=false, computeWorker=false, workerLimit=0;
let clockOrigin=performance.timeOrigin || 0;
const workers=new Set();
const idleWorkers=[];
function checkWorkers() { if(failure && Atomics.load(failure,0)) throw new Error("Ein WASM-Rechen-Worker ist fehlgeschlagen."); }
function failedWorker(error) {
  if(failure) {Atomics.store(failure,0,1);Atomics.notify(failure,0);}
  postMessage({type:"error",message:String(error?.message || error)});
}
function spawnCompute(stackTop,entry,done) {
  checkWorkers();
  if(!threaded || !idleWorkers.length) return 0;
  const worker=idleWorkers.pop();
  worker.completion=done>>>0;
  worker.postMessage({type:"run-compute",stackTop:stackTop>>>0,entry:entry>>>0,done:done>>>0});
  return 1;
}
async function prepareWorkers() {
  try {
    await Promise.all(Array.from({length:workerLimit},()=>new Promise((resolve,reject)=>{
      const worker=new Worker("worker.js",{type:"module"});workers.add(worker);
      const timer=setTimeout(()=>reject(new Error("Rechen-Worker konnte nicht starten.")),15000);
      worker.onerror=error=>{clearTimeout(timer);reject(error);if(ready)failedWorker(error);};
      worker.onmessage=({data})=>{
        if(data.type==="thread-ready") {clearTimeout(timer);idleWorkers.push(worker);resolve();}
        else if(data.type==="error") {clearTimeout(timer);reject(new Error(data.message));if(ready)failedWorker(data.message);}
        else if(data.type!=="thread-complete") postMessage(data);
      };
      worker.postMessage({type:"start-compute",module,memory,failure:failure.buffer,workerLimit,clockOrigin});
    })));
  } catch(error) {
    for(const worker of workers)worker.terminate();workers.clear();idleWorkers.length=0;
    throw error;
  }
}
const decode = new TextDecoder(), encode = new TextEncoder();
const littleEndian = new Uint8Array(new Uint32Array([1]).buffer)[0] === 1;
const buffers = [];
let pooledBytes = 0;
const bytes = (ptr, len) => new Uint8Array(memory.buffer, ptr >>> 0, len >>> 0);
function inbox(data) {
  const ptr = wasm.jfgui_buffer(data.length);
  if (!ptr && data.length) throw new Error("Eingabe überschreitet den verfügbaren Speicher.");
  bytes(ptr, data.length).set(data);
}
function tick() { wasm.jfgui_tick(Math.floor(performance.now()) >>> 0); }
function input() {
  // Older bundles remain usable with the shared host; rebuilt apps defer rasterization.
  if(wasm.jfgui_input) wasm.jfgui_input(Math.floor(performance.now()) >>> 0);
  else tick();
}
function pixelBuffer(length) {
  const index=buffers.findIndex(b=>b.byteLength===length);
  if(index<0) return new ArrayBuffer(length);
  const [buffer]=buffers.splice(index,1); pooledBytes-=length; return buffer;
}
function importFile(path, data, dropped, durable) {
  const name = encode.encode(path), content = new Uint8Array(data);
  const input = new Uint8Array(name.length + content.length);
  input.set(name); input.set(content, name.length); inbox(input);
  if (!wasm.jfgui_import(name.length, input.length, +dropped, +durable)) throw new Error(`Datei nicht importiert: ${path}`);
}
const imports = { jfgui: {
  write(stream, ptr, len) { postMessage({type:"log", stream, text:decode.decode(bytes(ptr, len).slice())}); },
  exit(code) { postMessage({type:"exit", code}); },
  time_us() { return ((performance.timeOrigin || 0)-clockOrigin+performance.now()) * 1000; },
  thread_count() { return workerLimit; },
  thread_reap(done) {
    for(const worker of workers) if(worker.completion===(done>>>0)) {worker.completion=undefined;idleWorkers.push(worker);break;}
  },
  thread_spawn: spawnCompute,
  thread_wait(address,expected,timeout) {
    checkWorkers();
    Atomics.wait(new Int32Array(memory.buffer), (address>>>0)/4, expected, Math.max(0,Math.min(timeout,100)));
    checkWorkers();
  },
  thread_notify(address,count) { Atomics.notify(new Int32Array(memory.buffer),(address>>>0)/4,count>>>0); },
  present(ptr, len, width, height, stride, rectsPtr=0, rectsCount=0) {
    ptr>>>=0;rectsPtr>>>=0;
    const source=bytes(ptr,len), patches=[];
    const rects=rectsCount ? new Int32Array(memory.buffer,rectsPtr,rectsCount*4) : [0,0,width,height];
    const words=littleEndian && !(ptr&3) && !(stride&3) ? new Uint32Array(memory.buffer,ptr,len>>>2) : null;
    for(let r=0;r<rects.length;r+=4) {
      const [x,y,w,h]=rects.subarray ? rects.subarray(r,r+4) : rects;
      const buffer=pixelBuffer(w*h*4);
      if(words) {
        const rgba=new Uint32Array(buffer);
        for(let row=0;row<h;row++) {
          let a=(y+row)*(stride>>>2)+x, b=row*w;
          for(let col=0;col<w;col++) {const c=words[a++];rgba[b++]=0xff000000|(c&0x0000ff00)|((c&255)<<16)|((c>>>16)&255);}
        }
      } else {
        const rgba=new Uint8ClampedArray(buffer);
        for(let row=0;row<h;row++) for(let col=0;col<w;col++) {
          const a=(y+row)*stride+(x+col)*4,b=(row*w+col)*4;
          rgba[b]=source[a+2];rgba[b+1]=source[a+1];rgba[b+2]=source[a];rgba[b+3]=255;
        }
      }
      patches.push({x,y,width:w,height:h,pixels:buffer});
    }
    postMessage({type:"frame",width,height,patches},patches.map(p=>p.pixels));
  },
  request(kind, id, ptr, len) {
    // Copy before returning to Rust: no async task borrows linear memory.
    const data = bytes(ptr, len).slice().buffer;
    postMessage({type:"service", kind, id, data}, [data]);
  },
}};
function dispatch(m) {
  switch(m.type) {
    case "tick": break;
    case "event": wasm.jfgui_event(...m.event); input(); break;
    case "text": {
      // Whole code points (never half a surrogate pair); jfgui_text answers the characters it
      // queued, the rest follows once input() handed them to the app (no rendering between).
      // Nothing taken twice in a row: no window takes text, the rest goes.
      const chars=Array.from(m.text);
      for(let i=0,idle=0;i<chars.length && idle<2;) {
        const data=encode.encode(chars.slice(i,i+4096).join(""));inbox(data);
        const n=wasm.jfgui_text(data.length);input();
        i+=n;idle=n?0:idle+1;
      }
      break;
    }
    case "import": importFile(m.path,m.bytes,m.dropped,m.durable);input();postMessage({type:"imported",id:m.id});break;
    case "clipboard": {
      const entries=[[1,m.text||""],...(m.html?[[4,m.html]]:[]),...(m.rtf?[[5,m.rtf]]:[])].map(([format,text])=>[format,encode.encode(text)]);
      const size=4+entries.reduce((n,[,v])=>n+8+v.length,0);
      if(size>48*1024*1024)throw new Error("Zwischenablage zu groß.");
      const data=new Uint8Array(size),view=new DataView(data.buffer);view.setUint32(0,entries.length,true);let at=4;
      for(const [format,value] of entries){view.setUint32(at,format,true);view.setUint32(at+4,value.length,true);data.set(value,at+8);at+=8+value.length;}
      inbox(data);if(!wasm.jfgui_clipboard(0,data.length))throw new Error("Zwischenablage nicht importiert.");break;
    }
    case "complete": wasm.jfgui_complete(m.id,m.status);input();break;
    // the answer of the system's picker (file_dialog): the bound path, or a negative status
    case "file": {
      const path=encode.encode(m.path||"");
      if(m.status===2 && wasm.jfgui_file_result) {inbox(path);wasm.jfgui_file_result(m.id,2,path.length);}
      else if(wasm.jfgui_file_result) wasm.jfgui_file_result(m.id,m.status,0);
      else wasm.jfgui_complete(m.id,m.status===2?-3:m.status);
      input();break;
    }
    // the accessibility mirror on / off (a11y.js), an action of assistive technology
    case "a11y": if(wasm.jfgui_a11y) {wasm.jfgui_a11y(+!!m.on);input();} break;
    case "a11y-act": {
      if(!wasm.jfgui_a11y_act) break;
      const text=encode.encode(String(m.text||""));
      if(text.length>1048576) throw new Error("Text für die Barrierefreiheit zu lang.");
      inbox(text);
      wasm.jfgui_a11y_act(m.win>>>0,m.lo>>>0,m.hi>>>0,m.act>>>0,m.a|0,m.b|0,Number(m.value)||0,text.length);input();break;
    }
    case "agent": wasm.jfgui_agent(+m.enabled);input();break;
    case "closeSession": {
      if(typeof m.session!=="string" || m.session.length>128) throw new Error("Ungültige Agentensitzung.");
      const data=encode.encode(JSON.stringify({session:m.session,close:true}));
      inbox(data);wasm.jfgui_control(data.length);
      postMessage({type:"rpc",id:m.id,result:null});break;
    }
    case "rpc": {
      // Finish preceding input before a state query, render resource or command.
      tick();
      const data=encode.encode(JSON.stringify({session:m.session||"page",request:m.request}));
      if(data.length>1048576) throw new Error("RPC-Anfrage überschreitet 1 MiB.");
      inbox(data);const len=wasm.jfgui_control(data.length);
      const result=len ? JSON.parse(decode.decode(bytes(wasm.jfgui_response(),len).slice())) : null;
      postMessage({type:"rpc",id:m.id,result});break;
    }
  }
}
self.onmessage = async ({data:m}) => {
  try {
    checkWorkers();
    if(m.type==="start-compute") {
      computeWorker=true;threaded=true;module=m.module;memory=m.memory;
      failure=new Int32Array(m.failure);workerLimit=m.workerLimit;
      clockOrigin=m.clockOrigin;
      ({exports:wasm}=await WebAssembly.instantiate(module,{...imports,env:{memory}}));
      postMessage({type:"thread-ready"});return;
    }
    if(m.type==="run-compute" && computeWorker) {
      wasm.__stack_pointer.value=m.stackTop;
      wasm.jfgui_thread_entry(m.entry);
      // Publish completion only AFTER WASM has stopped using its stack.
      const words=new Int32Array(memory.buffer);
      Atomics.store(words,m.done/4,1);Atomics.notify(words,m.done/4);
      postMessage({type:"thread-complete"});return;
    }
    if(m.type==="recycle") {
      for(const buffer of m.buffers) if(buffer instanceof ArrayBuffer && buffer.byteLength<=64*1024*1024) {
        while(buffers.length && (buffers.length>=16 || pooledBytes+buffer.byteLength>64*1024*1024)) pooledBytes-=buffers.shift().byteLength;
        buffers.push(buffer);pooledBytes+=buffer.byteLength;
      }
      return;
    }
    if (m.type === "start") {
      if (wasm) throw new Error("Worker wurde bereits gestartet.");
      let variants={scalar:m.url};
      try {const r=await fetch(new URL("variants.json",new URL(m.url,self.location.href)));if(r.ok) variants=await r.json();} catch {}
      // SIMD feature probe (a function returning v128.const); no user-agent sniffing.
      const simd=WebAssembly.validate(new Uint8Array([0,97,115,109,1,0,0,0,1,5,1,96,0,1,123,3,2,1,0,10,22,1,20,0,253,12,...Array(16).fill(0),11]));
      const canThread=simd && self.crossOriginIsolated && typeof SharedArrayBuffer!=="undefined";
      const mode=m.mode || "auto";
      const candidates=mode==="auto" ? [canThread&&"threads",simd&&"simd","scalar"].filter(Boolean) : [mode];
      let selected, lastError;
      for(const candidate of candidates) {
        if(!variants[candidate]) continue;
        if(candidate==="threads" && !canThread) {lastError=new Error("Paralleles WASM benötigt Cross-Origin-Isolation und SharedArrayBuffer.");continue;}
        try {
          const url=new URL(variants[candidate],new URL(m.url,self.location.href));
          const response=await fetch(url);
          if(!response.ok) throw new Error(`WASM HTTP ${response.status}`);
          module=await WebAssembly.compile(await response.arrayBuffer());
          threaded=candidate==="threads";
          if(threaded) {
            memory=new WebAssembly.Memory({initial:512,maximum:32768,shared:true});
            failure=new Int32Array(new SharedArrayBuffer(4));
            workerLimit=Math.max(1,Math.min(m.workers || Math.max(1,(navigator.hardwareConcurrency || 2)-1),8));
          }
          ({exports:wasm}=await WebAssembly.instantiate(module,threaded ? {...imports,env:{memory}} : imports));
          if(threaded) await prepareWorkers();
          selected=candidate;break;
        } catch(error) {lastError=error;threaded=false;failure=null;}
      }
      if(!selected) throw lastError || new Error("Kein passendes WASM-Bundle vorhanden.");
      memory = wasm.memory;
      wasm.jfgui_capabilities(m.capabilities);
      // the app's secrets (loader.js, secrets.js) before it starts; the inbox is wiped by Rust
      if (m.secrets && wasm.jfgui_secrets) { inbox(m.secrets); wasm.jfgui_secrets(m.secrets.length); m.secrets.fill(0); }
      for (const file of m.files) importFile(file.path, file.bytes, false, true);
      const now = new Date();
      wasm.jfgui_date(now.getFullYear(), now.getMonth()+1, now.getDate(), now.getHours(), now.getMinutes(), now.getSeconds(), now.getMilliseconds(), now.getDay());
      const langs = (navigator.languages && navigator.languages.length ? navigator.languages : [navigator.language || ""]).filter(Boolean);
      if (wasm.jfgui_env && langs.length) { const env = encode.encode("LANGUAGE="+langs.join(":")); inbox(env); wasm.jfgui_env(env.length); }
      const args = encode.encode(m.args.join("\n")); inbox(args);
      const code = wasm.jfgui_start(m.width, m.height, m.dpi, args.length);
      if (code) throw new Error(`Anwendung konnte nicht starten (${code}).`);
      ready=true; tick(); postMessage({type:"ready",compute:{mode:selected,simd:selected!=="scalar",shared:threaded,workers:workers.size}});
      return;
    }
    if (!ready) return;
    for(const message of m.type==="batch" ? m.messages : [m]) {
      try {dispatch(message);}
      catch(error) {postMessage({type:"error",id:message.id,message:String(error?.message || error)});}
    }
    tick();
    if(m.type==="tick") postMessage({type:"tick-complete"});
  } catch(error) {if(computeWorker) failedWorker(error);else postMessage({type:"error",id:m.id,message:String(error?.message || error)});}
  finally {if(m.type==="batch") postMessage({type:"batch-complete"});}
};
