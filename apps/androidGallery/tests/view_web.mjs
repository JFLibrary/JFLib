// (G5e2) A 3D view in the browser (ui.gpu_view on the page's WebGPU device, lib/ui/gpu_view.rs; docs/GPU.md "3D-Ansicht"):
// the page "3D-Ansicht" of the pocket gallery (pg_view.rs) runs its self-test (jfgui_GPU_VIEWCHECK=1) and says what it
// finds on the console: the view in the framebuffer against the same frame offscreen on the same device and against
// render3dtest's ray caster, at rest without settling by hand (the asynchronous read-back), below a popup, after a lost
// device.  This test reads those lines and looks from outside while the page holds still: the screen (page.screenshot,
// the browser's composition) = the framebuffer (ui.render) at rest and with the popup over the view; every presented
// image read back (jfgui_GPU_VERIFY).  Without a working WebGPU it says SKIP.
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/view_web.mjs [build] [--modes=simd,threads,scalar] [--save=DIR: the pictures of a difference]
// Playwright: tools/tests/playwright.mjs (PLAYWRIGHT_MODULE).
import {createServer} from 'node:http';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve, extname, sep, join} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright(), dir=resolve(build,'web','androidGallery');

const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(parts.length ? parts : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(await readFile(file));
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;

let id=0, failures=0, checks=0;
const say=(ok,what)=>{checks++;if(!ok)failures++;console.log(`${ok?'PASS':'FAIL'} ${what}`);};
async function rpc(page,method,params={}) {
  const r=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:++id,method,params});
  if(!r||r.error) throw new Error(JSON.stringify(r));
  return r.result;
}
async function counters(page) {
  const text=await page.evaluate(()=>window.jfguiAgent.gpuTest(0));
  return Object.fromEntries(text.split(' ').map(p=>{const k=p.indexOf('=');return [p.slice(0,k),isNaN(+p.slice(k+1))||p.slice(k+1)===''?p.slice(k+1):+p.slice(k+1)];}));
}
const settle=page=>page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(()=>setTimeout(r,60)))));
// the framebuffer's picture (ui.render: the agent's render, which settles what the GPU drew) as a PNG
async function framebufferPng(page) {
  const done=await rpc(page,'actions/execute',{action:'ui.render',input:{}});
  const uri=(done.resources||[])[0]?.uri||done.result?.image?.uri||done.value?.image?.uri;
  if(!uri) throw new Error(`ui.render gave no resource: ${JSON.stringify(done).slice(0,300)}`);
  let png=Buffer.alloc(0),offset=0;
  for(;;) {
    const chunk=await rpc(page,'resources/read',{uri,offset,limit:65536});
    png=Buffer.concat([png,Buffer.from(chunk.data,'base64')]);offset=chunk.nextOffset;
    if(chunk.eof)break;
  }
  return png;
}
// two PNGs in the page -> the pixels that differ (the first: its place), the largest difference
const diffPngs=(page,a,b,top=0)=>page.evaluate(async ([a64,b64,top])=>{
  const load=async s=>{
    const bm=await createImageBitmap(new Blob([Uint8Array.from(atob(s),c=>c.charCodeAt(0))],{type:'image/png'}));
    const c=new OffscreenCanvas(bm.width,bm.height),x=c.getContext('2d',{colorSpaceConversion:'none'});x.drawImage(bm,0,0);
    return {w:bm.width,h:bm.height,d:x.getImageData(0,0,bm.width,bm.height).data};
  };
  const p=await load(a64),q=await load(b64);
  if(p.w!==q.w||p.h+top>q.h) return {size:[p.w,p.h,q.w,q.h],pixels:-1};
  let n=0,first=null,most=0;
  const at=top*q.w*4;
  for(let i=0;i<p.d.length;i+=4) {
    const d=Math.max(Math.abs(p.d[i]-q.d[i+at]),Math.abs(p.d[i+1]-q.d[i+1+at]),Math.abs(p.d[i+2]-q.d[i+2+at]));
    if(d){if(!first)first=[(i/4)%p.w,Math.floor(i/4/p.w)];n++;if(d>most)most=d;}
  }
  return {pixels:n,first,top:most,size:[p.w,p.h]};
},[a.toString('base64'),b.toString('base64'),top]);
// the composed screen once it has come to rest
const shotOf=page=>page.screenshot({scale:'device'});
async function screenPng(page) {
  await counters(page);
  await settle(page);
  let last=await shotOf(page), same=0;
  for(let i=0;i<40&&same<2;i++) {
    await new Promise(r=>setTimeout(r,50));
    const next=await shotOf(page);
    same=next.equals(last)?same+1:0;
    last=next;
  }
  return last;
}
// the screen against the framebuffer: first the screen (the GPU's picture), then the framebuffer as it is, without a
// settle by hand in between that could hide what it lacks
async function screenEqualsFramebuffer(page,what) {
  const shot=await screenPng(page);
  const fb=await framebufferPng(page);
  const rows=await page.evaluate(()=>Math.round(document.querySelector('#screen').getBoundingClientRect().top*devicePixelRatio));
  const r=await diffPngs(page,fb,shot,rows);
  say(r.pixels===0,`${what}: the screen = the framebuffer ${JSON.stringify(r)}`);
  const keep=option('save',null);
  if(keep&&r.pixels) {
    const name=what.replace(/[^a-z0-9]+/gi,'_');
    await writeFile(join(keep,`${name}_screen.png`),shot);
    await writeFile(join(keep,`${name}_fb.png`),fb);
  }
}

let ran=0;
// --bench=N: the page's measurement instead (jfgui_GPU_VIEWBENCH: render3dtest's gallery in a view 700 px high, turning;
// N runs of 300 frames, one line each)
async function bench(browser,runs) {
  for(let k=0;k<runs;k++) {
    const context=await browser.newContext({locale:'de-DE',viewport:{width:1280,height:1000}});
    const page=await context.newPage();
    const got=new Promise(ok=>page.on('console',m=>{const t=m.text();if(t.startsWith('viewcheck: bench '))ok(t.slice(11));}));
    await page.mouse.move(1270,990);
    await page.goto(`${origin}/index.html?gpu=1&theme=light&compute=${option('modes','simd').split(',')[0]}&gpuenv=jfgui_GPU_VIEWBENCH=300`);
    console.log(`  run ${k+1}: ${await Promise.race([got,new Promise(ok=>setTimeout(()=>ok('no result in 120 s'),120000))])}`);
    await context.close();
  }
}

try {
  const browser=await pw.chromium.launch({headless:true,...chromiumOptions()});
  try {
    console.log(`--- chromium ${browser.version()}`);
    const runs=Number(option('bench','0'));
    if(runs>0) {
      await bench(browser,runs);
      ran++;
    }
    for(const mode of runs>0?[]:option('modes','simd,threads,scalar').split(',').filter(Boolean)) {
      const context=await browser.newContext({locale:'de-DE',viewport:{width:480,height:860}});
      const page=await context.newPage(), lines=[], errors=[], timeline=[];
      let hold=null, done=null;
      const holds=[];
      page.on('pageerror',e=>errors.push(String(e)));
      page.on('console',m=>{
        const t=m.text();
        // (the device the self-test loses on purpose says so)
        if(m.type()==='error'&&!/GPU device lost/.test(t)) {errors.push(t);timeline.push(t);}
        if(!t.startsWith('viewcheck: ')) return;
        const l=t.slice(11).trim();
        lines.push(l);
        timeline.push(`(the page) ${l.slice(0,80)}`);
        if(l.startsWith('hold ')) holds.push(l.slice(5));
        if(l.startsWith('done:')) done=l;
      });
      await page.addInitScript(()=>{window.__ready=false;addEventListener('jfgui-ready',()=>window.__ready=true);});
      // (the pointer away from every widget from the start: a hover fading would differ between two pictures)
      await page.mouse.move(470,850);
      await page.goto(`${origin}/index.html?agent=1&gpu=1&theme=light&compute=${mode}&gpuenv=jfgui_GPU_VERIFY=1,jfgui_GPU_VIEWCHECK=1,jfgui_GPU_RETRY_MS=300`);
      await page.waitForFunction(()=>window.__ready,null,{timeout:60000});
      await page.addStyleTag({content:'#input{opacity:0!important}'});
      await page.mouse.move(470,850);
      // (the agent's mark in the corner changes with its first request: before the first picture)
      await rpc(page,'state/inspect',{domain:'ui',limit:1}).catch(()=>{});
      const end=Date.now()+120000;
      let looked=0;
      while(!done&&Date.now()<end) {
        // (a hold: the page keeps still for 2.5 s; look at it once)
        if(holds.length>looked) {
          const what=holds[looked++];
          await screenEqualsFramebuffer(page,`${mode}: ${what}`);
        }
        await new Promise(r=>setTimeout(r,50));
      }
      const c=await counters(page);
      const skip=lines.find(l=>l.startsWith('the view:'))?.includes('device false');
      if(skip&&c.on!==1) {
        console.log(`SKIP ${mode}: no WebGPU device for the view here (${lines[1]})`);
        await context.close();
        continue;
      }
      ran++;
      for(const l of lines.filter(l=>l.startsWith('check ')||l.startsWith('the view:')||l.startsWith('a new device'))) console.log(`  ${mode}: ${l}`);
      for(const l of lines.filter(l=>/^check \d+: /.test(l))) say(/^check \d+: ok /.test(l),`${mode}: ${l.slice(0,110)}`);
      say(!!done&&/ 0 failed/.test(done),`${mode}: ${done||'the self-test did not end in 120 s'}`);
      say(c.verified>0&&c.verify_bad===0,`${mode}: presented images read back ${c.verified}, ${c.verify_bad} differ`);
      say(c.view_gen>=2&&c.view_attaches===2&&c.view_detaches===1,`${mode}: the lost device: generation ${c.view_gen}, ${c.view_attaches} attaches, ${c.view_detaches} detaches`);
      say(holds.length===2,`${mode}: ${holds.length} holds looked at`);
      say(errors.length===0,`${mode}: no error on the page`);
      if(errors.length) for(const e of timeline) console.log(`    ${mode}: ${e.trim().slice(0,200)}`);
      console.log(`  ${mode}: counters on=${c.on} adapter=${c.adapter} view_level=${c.view_level} view_apart=${c.view_apart} view_frames=${c.view_frames} view_reads=${c.view_reads} view_read_frame=${c.view_read_frame} view_quads=${c.view_quads} switches=${c.switches} leaves=${c.leaves}`);
      await context.close();
    }
  } finally {await browser.close();}
} finally {await new Promise(r=>server.close(r));}
if(!ran) console.log('SKIP: no browser here has a working WebGPU');
console.log(`${checks-failures} of ${checks} checks passed`);
if(failures) process.exitCode=1;
