// The UI's output through WebGPU in the browser (lib/ui/gpu_out.rs, lib/gpu/web_*.rs, web/gpu.js,
// lib/plat/web_gpu.rs; docs/GPU.md "Ausgabe im Browser"), with the real host and the real bundles.
//   - the output comes up while the app runs, in each of the three bundles; no frame on the way shows
//     anything but the app's picture (screenshots through the switch)
//   - what is on the screen (page.screenshot: the browser's composition) = the framebuffer (ui.render)
//     = the old path's picture of the same page, after hover, a menu, a dialog, a new size, a scale;
//     every presented image read back and compared (jfgui_GPU_VERIFY, GpuOutput::verified)
//   - layers apart: a menu has a texture of its own; a canvas pans from tiles, a big image becomes a
//     texture ("Bild & Leinwand" of apps/androidGallery, built with --web-mode simd), and at rest it
//     is the old path's picture
//   - fallback: no navigator.gpu, no adapter, a refused device, a browser whose WebGPU draws other
//     pixels, gpu.js not to be had: the old path, no canvas above #screen, no error on the page
//   - a lost device (device.destroy() from the test: a simulation, no driver reset): back to the
//     old path without a wrong frame, then a new device and a new canvas
// In a browser without a working WebGPU (Playwright's Firefox here: its build blocks WebGPU) the part
// that needs one says SKIP and the fallback test is what remains.
//   python build_rs.py app gallery --target web --release
//   node apps/gallery/tests/gpu_web.mjs [build] [--engines=chromium,firefox] [--modes=simd,threads,scalar]
// Playwright: tools/tests/playwright.mjs (PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright();
const dir=resolve(build,'web','gallery'), images=resolve(build,'web','androidGallery');

// what a browser without it looks like, put in front of the host's worker
const STUBS={
  nogpu:`Object.defineProperty(WorkerNavigator.prototype,"gpu",{get(){return undefined;}});`,
  noadapter:`GPU.prototype.requestAdapter=async function(){return null;};`,
  nodevice:`GPUAdapter.prototype.requestDevice=async function(){throw new DOMException("refused","OperationError");};`,
  // (the small picture finds it: nothing arrives in the textures)
  badpixels:`GPUQueue.prototype.writeTexture=function(){};`,
  badshader:`{const c=GPUDevice.prototype.createShaderModule;GPUDevice.prototype.createShaderModule=function(d){return c.call(this,{code:"this is no WGSL"});};}`,
};
const server=createServer(async (request,response)=>{
  try {
    const url=new URL(request.url,'http://localhost');
    const parts=decodeURIComponent(url.pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    // /isolated/... : the headers the threads bundle needs; /STUB/... : a stub before worker.js
    const first=parts.shift()||'';
    if(first==='gone'&&parts[0]==='gpu.js') {response.writeHead(404).end();return;}
    const root=first==='images'?images:dir;
    const file=resolve(root,...(parts.length ? parts : ['index.html']));
    if(!file.startsWith(root+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    let body=await readFile(file);
    if(STUBS[first]&&parts[0]==='worker.js') body=Buffer.concat([Buffer.from(STUBS[first]+'\n'),body]);
    const isolate=first==='isolated'?{'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}:{};
    response.writeHead(200,{'content-type':type,'cache-control':'no-store',...isolate}).end(body);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;

let id=0, failures=0, checks=0;
const say=(ok,what)=>{checks++;if(!ok)failures++;console.log(`${ok?'PASS':'FAIL'} ${what}`);};
async function rpc(page,method,params={}) {
  const r=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:++id,method,params});
  assert.ok(r&&!r.error,JSON.stringify(r));
  return r.result;
}
async function counters(page) {
  const text=await page.evaluate(()=>window.jfguiAgent.gpuTest(0));
  return Object.fromEntries(text.split(' ').map(p=>{const k=p.indexOf('=');return [p.slice(0,k),isNaN(+p.slice(k+1))||p.slice(k+1)===''?p.slice(k+1):+p.slice(k+1)];}));
}
async function until(fn,ok,what,ms=15000) {
  const end=Date.now()+ms;
  let v;
  do {v=await fn();if(ok(v))return v;await new Promise(r=>setTimeout(r,50));} while(Date.now()<end);
  throw new Error(`timed out: ${what}; last ${JSON.stringify(v)}`);
}
const settle=page=>page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(r,60)))));
// the framebuffer's picture, the page's screen and the 2D canvas as RGBA arrays in the page
async function framebufferPng(page) {
  const done=await rpc(page,'actions/execute',{action:'ui.render',input:{}});
  const uri=(done.resources||[])[0]?.uri||done.result?.image?.uri||done.value?.image?.uri;
  assert.ok(uri,`ui.render gave a resource: ${JSON.stringify(done).slice(0,300)}`);
  let png=Buffer.alloc(0),offset=0;
  for(;;) {
    const chunk=await rpc(page,'resources/read',{uri,offset,limit:65536});
    png=Buffer.concat([png,Buffer.from(chunk.data,'base64')]);offset=chunk.nextOffset;
    if(chunk.eof)break;
  }
  return png;
}
// two PNGs in one page -> the pixels that differ (the first: its place), the largest difference
const diffPngs=(page,a,b)=>page.evaluate(async ([a64,b64])=>{
  const load=async s=>{
    const bm=await createImageBitmap(new Blob([Uint8Array.from(atob(s),c=>c.charCodeAt(0))],{type:'image/png'}));
    const c=new OffscreenCanvas(bm.width,bm.height),x=c.getContext('2d',{colorSpaceConversion:'none'});x.drawImage(bm,0,0);
    return {w:bm.width,h:bm.height,d:x.getImageData(0,0,bm.width,bm.height).data};
  };
  const p=await load(a64),q=await load(b64);
  if(p.w!==q.w||p.h!==q.h) return {size:[p.w,p.h,q.w,q.h],pixels:-1};
  let n=0,first=null,top=0;
  for(let i=0;i<p.d.length;i+=4) {
    const d=Math.max(Math.abs(p.d[i]-q.d[i]),Math.abs(p.d[i+1]-q.d[i+1]),Math.abs(p.d[i+2]-q.d[i+2]));
    if(d){if(!first)first=[(i/4)%p.w,Math.floor(i/4/p.w)];n++;if(d>top)top=d;}
  }
  return {pixels:n,first,top,size:[p.w,p.h]};
},[a.toString('base64'),b.toString('base64')]);
// the composed screen at the canvas, at its own pixels, once it has come to rest (hover fades, tooltips)
const shotOf=async page=>page.screenshot({clip:await page.locator('#screen').boundingBox(),scale:'device'});
const screenPng=async page=>{
  await settle(page);
  let last=await shotOf(page);
  for(let i=0;i<40;i++) {
    await new Promise(r=>setTimeout(r,50));
    const next=await shotOf(page);
    if(next.equals(last)) return next;
    last=next;
  }
  return last;
};
const compareScreen=async (page,what)=>{
  const fb=await framebufferPng(page);
  const shot=await screenPng(page);
  const r=await diffPngs(page,fb,shot);
  say(r.pixels===0,`${what}: screen = framebuffer ${JSON.stringify(r)}`);
  return r.pixels;
};
const openPage=async (context,url)=>{
  const page=await context.newPage(), errors=[];
  page.on('pageerror',e=>errors.push(String(e)));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  await page.addInitScript(()=>{window.__ready=false;addEventListener('jfgui-ready',()=>window.__ready=true);});
  await page.goto(url);
  await page.waitForFunction(()=>window.__ready,null,{timeout:60000});
  // (#input, which keeps the keyboard, lies over the canvas at 1 % opacity: its pixels are the page's on both paths)
  await page.addStyleTag({content:'#input{opacity:0!important}'});
  return {page,errors};
};
// what both paths do, step by step -> the picture of each step
async function steps(page,prefix,check) {
  const box=await page.locator('#screen').boundingBox();
  const out={};
  const step=async name=>{out[name]=await screenPng(page);if(check)await check(name,out[name]);};
  await step('first frame');
  for(let i=0;i<12;i++)await page.mouse.move(box.x+40+i*31,box.y+120+(i*47)%300);
  await step('hover');
  await page.mouse.click(box.x+30,box.y+52);
  for(let i=0;i<4;i++)await page.mouse.move(box.x+50,box.y+80+i*20);
  await step('menu open');
  await page.keyboard.press('Escape');
  await page.mouse.move(box.x+500,box.y+300);
  await step('menu closed');
  await page.setViewportSize({width:1001,height:713});
  await page.waitForFunction(()=>document.querySelector('#screen').width===Math.round(document.querySelector('#screen').clientWidth*devicePixelRatio));
  await page.waitForTimeout(300);
  await page.mouse.move(box.x+200,box.y+200);
  await step('new size');
  return out;
}
const hasCanvas=page=>page.evaluate(()=>!!document.querySelector('#gpu'));

// ---- the fallback: each way to have no WebGPU, the old path, nothing above #screen
async function fallback(browser,engine) {
  for(const [stub,want] of [['nogpu','no navigator.gpu'],['noadapter','no adapter'],['nodevice','a refused device'],['gone','gpu.js not to be had'],['badpixels','WebGPU draws other pixels'],['badshader','a shader the browser refuses']]) {
    const context=await browser.newContext({locale:'de-DE',viewport:{width:800,height:600}});
    try {
      const {page,errors}=await openPage(context,`${origin}/${stub}/index.html?agent=1&gpu=1&theme=light`);
      // (it has given up, or it never starts)
      const gives_up=stub==='nodevice' ? x=>x.error!=='' : x=>x.never===1; // (a refused device may be for now: it tries again later)
      const c=await until(()=>counters(page),gives_up,`${stub}: the output gives up`,12000).catch(async e=>{console.log(`   ${e.message}`);return counters(page);});
      const canvas=await hasCanvas(page);
      const fb=await framebufferPng(page);
      const canvasEqual=await page.evaluate(async b64=>{
        const bm=await createImageBitmap(new Blob([Uint8Array.from(atob(b64),c=>c.charCodeAt(0))],{type:'image/png'}));
        const f=document.createElement('canvas');f.width=bm.width;f.height=bm.height;const x=f.getContext('2d');x.drawImage(bm,0,0);
        const s=document.querySelector('#screen');
        if(s.width!==bm.width||s.height!==bm.height) return -1;
        const p=x.getImageData(0,0,f.width,f.height).data,q=s.getContext('2d').getImageData(0,0,f.width,f.height).data;
        let n=0;for(let i=0;i<p.length;i+=4)if(p[i]!==q[i]||p[i+1]!==q[i+1]||p[i+2]!==q[i+2])n++;
        return n;
      },fb.toString('base64'));
      // (a stub's own errors reach the console as warnings at most)
      say(c.on===0&&gives_up(c)&&!canvas&&canvasEqual===0&&errors.length===0,`${engine} ${want}: the old path, #screen = framebuffer (${canvasEqual} pixels differ), canvas above: ${canvas}, never=${c.never} error=${c.error} page errors: ${errors.length}`);
    } finally {await context.close();}
  }
}

// ---- the whole path in one bundle
async function path(browser,engine,mode,scale) {
  const folder=mode==='threads'?'isolated':'plain';
  const options={locale:'de-DE',viewport:{width:960,height:640},deviceScaleFactor:scale};
  const label=`${engine} ${mode} ${scale*100} %`;
  // the old path of the same page, step by step, as the reference
  const refContext=await browser.newContext(options);
  let ref;
  let refFirst;
  try {
    const old=await openPage(refContext,`${origin}/${folder}/index.html?agent=1&gpu=0&theme=light&compute=${mode}`);
    assert.equal((await counters(old.page)).on,0);
    say(!await hasCanvas(old.page),`${label}: ?gpu=0: no canvas above #screen`);
    refFirst=await screenPng(old.page);
    ref=await steps(old.page);
  } finally {await refContext.close();}

  const context=await browser.newContext(options);
  try {
    const url=`${origin}/${folder}/index.html?agent=1&gpu=1&theme=light&compute=${mode}&gpuenv=jfgui_GPU_VERIFY=1,jfgui_GPU_RETRY_MS=200`;
    const {page,errors}=await openPage(context,url);
    assert.equal(await page.evaluate(()=>window.__jfguiCompute.mode),mode,'the bundle asked for');
    // through the switch: every picture until the canvas shows is the app's, none black or empty
    const shots=[];
    let c=await counters(page);
    for(let i=0;i<100&&c.on!==1;i++) {
      shots.push(await shotOf(page));
      c=await counters(page);
      if(c.never===1) break;
    }
    if(c.never===1) {console.log(`SKIP ${label}: no working WebGPU here (${c.error})`);return false;}
    c=await until(()=>counters(page),x=>x.on===1,`${mode}: on the canvas`);
    for(let i=0;i<4;i++) shots.push(await shotOf(page));
    let wrong=0;
    for(const shot of shots) if((await diffPngs(page,refFirst,shot)).pixels!==0) wrong++;
    say(wrong===0,`${label}: through the switch ${shots.length} pictures, ${wrong} differ from the old path's; adapter ${c.adapter}, ${c.present_mode}`);
    say(await hasCanvas(page),`${label}: a canvas above #screen`);
    let total=0, againstFb=0;
    const firstBefore=await counters(page);
    const shown=await steps(page,label,async (name,shot)=>{
      const r=await diffPngs(page,ref[name],shot);
      total+=r.pixels;
      say(r.pixels===0,`${label}: ${name}: screen = the old path's screen ${JSON.stringify(r)}`);
      // (the running counters: the menu is a layer of its own)
      if(name==='menu open') {
        const menu=await counters(page);
        say(menu.layer_textures>=1&&menu.split_frames>firstBefore.split_frames,`${label}: the menu is a layer apart (layer_textures=${menu.layer_textures}, split_frames ${firstBefore.split_frames} -> ${menu.split_frames})`);
      }
      if(name==='new size') {
        const resized=await until(()=>counters(page),x=>x.on===1&&x.leaves>firstBefore.leaves,'back on the canvas after the new size');
        say(resized.switches>firstBefore.switches&&await hasCanvas(page),`${label}: a new canvas after the new size (leaves=${resized.leaves}, switches=${resized.switches})`);
      }
    });
    assert.ok(shown);
    againstFb+=await compareScreen(page,`${label}: at rest`);
    // ---- what was presented, read back
    const v=await until(()=>counters(page),x=>x.verified>0,'presented images read back',5000).catch(()=>counters(page));
    say(v.verified>0&&v.verify_bad===0,`${label}: presented images read back ${v.verified}, ${v.verify_bad} differ`);
    say(errors.length===0,`${label}: no error on the page ${errors.join('; ')}`);
    // ---- a lost device (simulated: device.destroy())
    const box=await page.locator('#screen').boundingBox();
    const lost=await counters(page);
    await page.evaluate(()=>window.jfguiAgent.gpuTest(1));
    const off=await until(()=>counters(page),x=>x.on===0,'back on the CPU path after the lost device',3000);
    const gone=await until(()=>hasCanvas(page),x=>!x,'the canvas gone',3000);
    assert.equal(gone,false);
    againstFb+=await compareScreen(page,`${label}: lost device: on the old path`);
    await page.mouse.move(box.x+300,box.y+220);
    againstFb+=await compareScreen(page,`${label}: lost device: a frame on the old path`);
    const back=await until(()=>counters(page),x=>x.on===1,'a new device',10000);
    say(back.switches>lost.switches&&await hasCanvas(page),`${label}: lost device (simulated): error=${off.error}, a new device and canvas (switches ${lost.switches} -> ${back.switches})`);
    await page.mouse.move(box.x+320,box.y+240);
    againstFb+=await compareScreen(page,`${label}: lost device: the new device's frame`);
    const after=await until(()=>counters(page),x=>x.verified>lost.verified,'read back on the new device',5000).catch(()=>counters(page));
    say(after.verify_bad===0,`${label}: after the new device: read back ${after.verified}, ${after.verify_bad} differ`);
    console.log(`     ${label}: against the old path ${total} pixels differ, against the framebuffer ${againstFb}`);
    return true;
  } finally {await context.close();}
}

// ---- a canvas from tiles and an image as a texture ("Bild & Leinwand"), against the old path at rest
async function canvasAndImage(browser,engine) {
  const go=async (page,record)=>{
    // (the menu "Seiten": the topmost widget of that name)
    const box=await page.locator('#screen').boundingBox();
    const clickOn=async label=>{
      const {widgets}=await rpc(page,'state/inspect',{domain:'ui',limit:800});
      const w=widgets.filter(x=>x.label===label).sort((a,b)=>a.layer-b.layer).at(-1);
      assert.ok(w,`a widget "${label}"`);
      await page.mouse.click(box.x+w.rect[0]+w.rect[2]/2,box.y+w.rect[1]+w.rect[3]/2);
      await page.waitForTimeout(300);
    };
    await clickOn('Seiten');
    await clickOn('Bild & Leinwand');
    await page.waitForTimeout(500);
    const at=[box.x+box.width/2,box.y+230];
    const out={};
    await page.mouse.move(...at);
    await page.mouse.down();
    let during=null;
    for(let i=1;i<=12;i++) {
      await page.mouse.move(at[0]+i*7,at[1]+i*3);
      await page.waitForTimeout(16);
      if(i===10&&record) during=await record('drag');
    }
    await page.mouse.up();
    await page.waitForTimeout(400); // (the CPU's picture follows the gesture)
    out.drag=await screenPng(page);
    // the page scrolled, far enough for the picture below: rent or buy
    await page.mouse.move(box.x+box.width-60,box.y+box.height-40);
    for(let i=0;i<60;i++) {await page.mouse.wheel(0,i%20<12?80:-80);await page.waitForTimeout(16);}
    for(let i=0;i<40;i++) {await page.mouse.wheel(0,i%2?60:-60);await page.waitForTimeout(16);}
    if(record) await record('image');
    await page.waitForTimeout(300);
    out.scroll=await screenPng(page);
    return {out,during};
  };
  const options={locale:'de-DE',viewport:{width:960,height:900}};
  const a=await browser.newContext(options);
  let ref;
  try {
    const old=await openPage(a,`${origin}/images/index.html?agent=1&gpu=0&theme=light&compute=simd`);
    ref=(await go(old.page)).out;
  } finally {await a.close();}
  const b=await browser.newContext(options);
  try {
    const {page,errors}=await openPage(b,`${origin}/images/index.html?agent=1&gpu=1&theme=light&compute=simd&gpuenv=jfgui_GPU_VERIFY=1`);
    const on=await until(()=>counters(page),x=>x.on===1||x.never===1,'the output decides');
    if(on.never===1) {console.log(`SKIP ${engine} canvas and image: no working WebGPU here`);return;}
    let tiles=null, image=null;
    const {out}=await go(page,async what=>{
      const c=await counters(page);
      if(what==='drag') tiles=c;
      else image=c;
      return c;
    });
    say(tiles.canvas_apart>=1&&tiles.canvas_tiles>0,`${engine}: a canvas pans from tiles (apart=${tiles.canvas_apart}, tiles=${tiles.canvas_tiles}, fills=${tiles.canvas_fills})`);
    const d1=await diffPngs(page,ref.drag,out.drag);
    say(d1.pixels===0,`${engine}: after the gesture: screen = the old path's screen ${JSON.stringify(d1)}`);
    say(image.image_textures>=1,`${engine}: a big image is a texture (image_textures=${image.image_textures}, quads=${image.image_quads}, split_frames=${image.split_frames})`);
    const d2=await diffPngs(page,ref.scroll,out.scroll);
    say(d2.pixels===0,`${engine}: scrolled over the image, at rest: screen = the old path's screen ${JSON.stringify(d2)}`);
    await compareScreen(page,`${engine}: canvas and image: at rest`);
    const v=await until(()=>counters(page),x=>x.pending===0,'nothing on its way',3000).catch(()=>counters(page));
    say(v.verified>0&&v.verify_bad===0,`${engine}: canvas and image: read back ${v.verified}, ${v.verify_bad} differ; during the gesture ${v.canvas_diff_px} pixels of the tiles differed from the CPU's, by ${v.canvas_diff_max} at most`);
    say(errors.length===0,`${engine}: canvas and image: no error on the page ${errors.join('; ')}`);
  } finally {await b.close();}
}

try {
  const engines=option('engines','chromium,firefox').split(',').filter(Boolean);
  const modes=option('modes','simd,threads,scalar').split(',').filter(Boolean);
  for(const engine of engines) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      console.log(`--- ${engine} ${browser.version()}`);
      let any=false;
      for(const mode of modes) for(const scale of mode==='simd'?[1,2]:[1]) any=await path(browser,engine,mode,scale)||any;
      if(!any) console.log(`SKIP ${engine}: the path without a working WebGPU; the fallback is what this browser tests`);
      else await canvasAndImage(browser,engine);
      await fallback(browser,engine);
    } finally {await browser.close();}
  }
} finally {await new Promise(r=>server.close(r));}
console.log(`${checks-failures} of ${checks} checks passed`);
if(failures) process.exitCode=1;
