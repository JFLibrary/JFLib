// What presenting costs in the browser worker (lib/plat/web_windows.rs, web/worker.js, web/loader.js)
// and that the canvas gets the same pixels.
// 1. The app's real WASM in a Node vm (like web_frames.mjs) at 1920 x 1080 and 3840 x 2160: whole
//    frames (the size toggles by one row), hover frames (small damage) and the same over an open
//    menu (a popup: composited).  Per frame the time in the platform's present paths
//    (jfgui_present_us: the window copies, the composition, the host's present import) and in
//    the import alone; the canvas rebuilt from the frame messages must equal a forced full
//    composition, and with --base the canvas of the other build after the same input.
// 2. Chromium and Firefox (Playwright, tools/tests/playwright.mjs; --no-browsers skips them) at
//    100 % and 200 %: after hovering, opening and closing a menu and a new viewport size the
//    canvas (getImageData) equals the framebuffer (the agent's ui.render).
//   python build_rs.py app gallery --target web --web-mode simd
//   node tools/tests/web_present.mjs build [--base build/base] [--app gallery] [--frames 40] [--no-browsers]
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {resolve} from 'node:path';
import vm from 'node:vm';
import {createHash} from 'node:crypto';
import {performance} from 'node:perf_hooks';

const args=process.argv.slice(2);
const option=(name,fallback)=>{const i=args.indexOf(`--${name}`);return i>=0?args[i+1]:fallback;};
const builds={new:resolve(args.find((a,i)=>!a.startsWith('--')&&!args[i-1]?.startsWith('--'))||'build')};
if(option('base'))builds.base=resolve(option('base'));
const app=option('app','gallery'),count=Number(option('frames','40'));

async function appWorker(build,width,height){
  // the app's clock is virtual (frames the same in every run); inside a WASM call time_us
  // reads the real clock, so the present paths measure themselves
  let now=1000,depth=0,pixels=new Uint8Array(),size=[0,0],frames=0,importUs=0,exports=null;
  const errors=[],recycled=[];
  const wasmApi={Module:WebAssembly.Module,Memory:WebAssembly.Memory,Instance:WebAssembly.Instance,
    compile:b=>WebAssembly.compile(b),validate:b=>WebAssembly.validate(b),
    instantiate:async(module,imports)=>{
      const present=imports.jfgui.present;
      imports.jfgui.present=(...a)=>{const t=performance.now();present(...a);importUs+=(performance.now()-t)*1000;};
      const r=await WebAssembly.instantiate(module,imports);
      const real=r.exports?r.exports:r.instance.exports;
      exports={};
      for(const [k,v] of Object.entries(real))exports[k]=typeof v==='function'?(...a)=>{depth++;try{return v(...a);}finally{depth--;}}:v;
      return {exports,instance:{exports}};
    }};
  const sandbox={URL,TextDecoder,TextEncoder,Uint8Array,Uint8ClampedArray,Uint32Array,Int32Array,ArrayBuffer,DataView,WebAssembly:wasmApi,Date,console,
    navigator:{languages:['en-US'],language:'en-US',hardwareConcurrency:4},
    location:{href:`http://localhost/${app}/worker.js`},performance:{timeOrigin:0,now:()=>depth?performance.now():now},fetch:async url=>{
      const b=await readFile(resolve(build,'web',app,new URL(url,`http://localhost/${app}/`).pathname.split('/').at(-1)));
      return {ok:true,arrayBuffer:async()=>b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength),json:async()=>JSON.parse(b.toString())};
    },postMessage:m=>{
      if(m.type==='frame'){
        frames++;
        if(size[0]!==m.width||size[1]!==m.height){size=[m.width,m.height];pixels=new Uint8Array(m.width*m.height*4);}
        for(const p of m.patches||[{x:0,y:0,width:m.width,height:m.height,pixels:m.pixels}]){
          const source=m.buffer?new Uint8Array(m.buffer,p.offset,p.width*p.height*4):new Uint8Array(p.pixels);
          for(let y=0;y<p.height;y++)pixels.set(source.subarray(y*p.width*4,(y+1)*p.width*4),((p.y+y)*m.width+p.x)*4);
          if(!m.buffer)recycled.push(p.pixels);
        }
        if(m.buffer)recycled.push(m.buffer);
      }
      if(m.type==='error'||m.type==='exit')errors.push(m);
    }};
  sandbox.self=sandbox;
  vm.runInNewContext(await readFile(resolve(build,'web',app,'worker.js'),'utf8'),sandbox);
  async function send(data){await sandbox.onmessage({data});if(recycled.length)await sandbox.onmessage({data:{type:'recycle',buffers:recycled.splice(0)}});assert.deepEqual(errors,[]);}
  await send({type:'start',url:`${app}.wasm`,width,height,dpi:96,args:['--theme','light','--anim','off'],files:[],capabilities:4});
  return {send,advance:ms=>{now+=ms;},pixels:()=>pixels,size:()=>size,
    hash:()=>createHash('sha256').update(pixels).digest('hex'),
    counters:()=>({frames,importUs,presentUs:exports.jfgui_present_us()})};
}
const event=(type,x,y,button=0,data=0)=>({type:'event',event:[type,button,0,0,x,y,data>>>0]});

const results={};
for(const [width,height] of [[1920,1080],[3840,2160]]){
  const runs={};
  for(const [name,build] of Object.entries(builds)){
    const w=await appWorker(build,width,height);
    await w.send({type:'tick'});
    const measure=async(kind,step)=>{
      const a=w.counters();
      for(let i=0;i<count;i++){w.advance(16);await step(i);}
      const b=w.counters(),n=Math.max(1,b.frames-a.frames);
      return {kind,frames:b.frames-a.frames,presentUs:Math.round((b.presentUs-a.presentUs)/n),importUs:Math.round((b.importUs-a.importUs)/n)};
    };
    // whole frames: the canvas one row smaller / larger (a full render and composition)
    const full=await measure('full',i=>w.send({type:'event',event:[8,0,0,0,width,height-1-(i%2),0]}));
    await w.send({type:'event',event:[8,0,0,0,width,height,0]});
    // the pointer over the widgets (small damage)
    const hover=await measure('hover',i=>w.send(event(1,60+(i*29)%500,120+(i*41)%600)));
    // the canvas rebuilt from partial frames equals a forced full composition
    const fullCheck=async what=>{
      const partial=w.hash();
      await w.send({type:'event',event:[8,0,0,0,width,height,0]});
      assert.equal(w.hash(),partial,`${name} ${width}x${height}: ${what}: partial frames equal a full composition`);
      return partial;
    };
    await fullCheck('one window');
    // the menu "Datei" (a popup: the windows and popups are composited), the pointer over it
    await w.send(event(1,30,52));await w.send(event(2,30,52,1));await w.send(event(3,30,52,1));
    const menu=await measure('menu',i=>w.send(event(1,40+(i%3)*20,80+(i*23)%160)));
    const withMenu=await fullCheck('menu open');
    await w.send({type:'event',event:[5,0,0,0,0,0,27]});await w.send({type:'event',event:[6,0,0,0,0,0,27]});
    for(let i=0;i<4;i++){w.advance(16);await w.send(event(1,300+i*40,250));}
    const closed=await fullCheck('menu closed');
    assert.notEqual(withMenu,closed,`${name}: the menu opened and closed`);
    runs[name]={full,hover,menu,hash:closed,size:w.size()};
    console.log(JSON.stringify({build:name,width,height,full,hover,menu}));
  }
  if(runs.base)assert.equal(runs.new.hash,runs.base.hash,`${width}x${height}: the canvas of both builds is the same after the same input`);
  results[`${width}x${height}`]=runs;
}
console.log(`PASS web_present: ${Object.keys(builds).join(' + ')}, canvases ${builds.base?'identical, ':''}partial = full composition`);

// ---- the canvas in real browsers against the framebuffer
if(!args.includes('--no-browsers')){
  const {createServer}=await import('node:http');
  const {extname,sep}=await import('node:path');
  const {loadPlaywright,chromiumOptions}=await import('./playwright.mjs');
  process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
  const pw=loadPlaywright(),dir=resolve(builds.new,'web',app);
  const server=createServer(async(request,response)=>{try{
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico'){response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length?path:['index.html']));
    if(!file.startsWith(dir+sep))throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)]||'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store'}).end(await readFile(file));
  }catch(error){response.writeHead(404).end(String(error));}});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));
  const base=`http://127.0.0.1:${server.address().port}/index.html?agent=1&theme=light`;
  let id=0,failures=0;
  const rpc=async(page,method,params={})=>{
    const r=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:++id,method,params});
    assert.ok(r&&!r.error,JSON.stringify(r));return r.result;
  };
  // the framebuffer (ui.render's PNG) against the canvas -> differing pixels
  const compare=async page=>{
    const done=await rpc(page,'actions/execute',{action:'ui.render',input:{}});
    const uri=(done.resources||[])[0]?.uri||done.result?.image?.uri||done.value?.image?.uri;
    assert.ok(uri,`ui.render gave a resource: ${JSON.stringify(done).slice(0,300)}`);
    let png=Buffer.alloc(0),offset=0;
    for(;;){
      const chunk=await rpc(page,'resources/read',{uri,offset,limit:65536});
      png=Buffer.concat([png,Buffer.from(chunk.data,'base64')]);offset=chunk.nextOffset;
      if(chunk.eof)break;
    }
    await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));
    return page.evaluate(async b64=>{
      const bytes=Uint8Array.from(atob(b64),c=>c.charCodeAt(0));
      const bitmap=await createImageBitmap(new Blob([bytes],{type:'image/png'}));
      const fb=document.createElement('canvas');fb.width=bitmap.width;fb.height=bitmap.height;
      const fctx=fb.getContext('2d');fctx.drawImage(bitmap,0,0);bitmap.close();
      const screen=document.querySelector('#screen');
      if(fb.width!==screen.width||fb.height!==screen.height)return {size:[fb.width,fb.height,screen.width,screen.height]};
      const a=fctx.getImageData(0,0,fb.width,fb.height).data,b=screen.getContext('2d').getImageData(0,0,fb.width,fb.height).data;
      let n=0,first=-1;
      for(let i=0;i<a.length;i+=4)if(a[i]!==b[i]||a[i+1]!==b[i+1]||a[i+2]!==b[i+2]){if(first<0)first=i/4;n++;}
      return {pixels:n,first:first<0?null:[first%fb.width,Math.floor(first/fb.width)],size:[fb.width,fb.height]};
    },png.toString('base64'));
  };
  try{
    for(const engine of ['chromium','firefox']){
      const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
      try{
        for(const scale of [1,2]){
          const context=await browser.newContext({viewport:{width:960,height:640},deviceScaleFactor:scale});
          const page=await context.newPage(),errors=[];
          page.on('pageerror',e=>errors.push(String(e)));
          try{
            await page.addInitScript(()=>{window.__ready=false;addEventListener('jfgui-ready',()=>window.__ready=true);});
            await page.goto(base);await page.waitForFunction(()=>window.__ready,{timeout:45000});
            const canvas=await page.locator('#screen').boundingBox();
            const steps=[];
            for(let i=0;i<12;i++)await page.mouse.move(canvas.x+40+i*31,canvas.y+120+(i*47)%300);
            steps.push(['hover',await compare(page)]);
            await page.mouse.click(canvas.x+30,canvas.y+52);
            for(let i=0;i<6;i++)await page.mouse.move(canvas.x+50,canvas.y+80+i*20);
            await page.keyboard.press('Escape');
            await page.mouse.move(canvas.x+500,canvas.y+300);
            steps.push(['menu opened and closed',await compare(page)]);
            await page.setViewportSize({width:1001,height:713});
            await page.waitForTimeout(300);
            await page.mouse.move(canvas.x+200,canvas.y+200);
            steps.push(['new viewport size',await compare(page)]);
            for(const [what,r] of steps){
              const ok=r.pixels===0;
              if(!ok)failures++;
              console.log(`${ok?'PASS':'FAIL'} ${engine} ${scale*100} %: ${what}: canvas = framebuffer ${JSON.stringify(r)}`);
            }
            assert.deepEqual(errors,[]);
          }finally{await context.close();}
        }
      }finally{await browser.close();}
    }
  }finally{await new Promise(r=>server.close(r));}
  if(failures)process.exitCode=1;
}
