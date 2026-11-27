// The shared browser host's pacing (web/loader.js, web/worker.js): how fast a view that draws through ui.timer(id, 1)
// runs, and how often an app at rest or with a slow timer is woken (batches a second), in Chromium and Firefox.
//   python build_rs.py app gamedemo --target web
//   python build_rs.py app screentest --target web
//   node tools/tests/web_pacing.mjs [build] [--secs=5] [--browsers=chromium,firefox] [--host=DIR] [--max-idle=N]
// Cases: gamedemo at rest (paused, before the click that starts it), gamedemo's view turning by itself
// (?gpuenv=jfgui_GPU_GAMEBENCH: the frames the app drew, its RESULT lines), screentest at rest and with the caret
// blinking in its text box (ui.focus through the agent interface: a timer of 530 ms).  --host=DIR serves loader.js and
// worker.js from DIR instead of the build's copies (the host before a change, from git show).  --max-idle=N fails
// when an app at rest or with the caret is woken more than N times a second (the check of the build's host).
// Playwright: tools/tests/playwright.mjs (PLAYWRIGHT_MODULE; Firefox below PLAYWRIGHT_BROWSERS_PATH).
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from './playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.slice(name.length+3) ?? fallback;
const secs=Number(option('secs','5')), host=option('host',''), maxIdle=Number(option('max-idle','0'));
const engines=option('browsers','chromium,firefox').split(',').filter(Boolean);
const pw=loadPlaywright(), web=resolve(build,'web');

const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    if(parts.length===1) parts.push('index.html');
    const own=host && parts.length===2 && ['loader.js','worker.js'].includes(parts[1]);
    const root=own ? resolve(host) : web, file=own ? resolve(root,parts[1]) : resolve(root,...parts);
    if(!file.startsWith(root+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(await readFile(file));
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;

// counts what the page and the worker say to each other: batches (each one a tick of the app), the frames of the CPU path
function instrument() {
  const stats=window.__pacing={batches:0,frames:0,raf:0};
  const Base=window.Worker;
  window.Worker=class extends Base {
    constructor(...a) {
      super(...a);
      this.addEventListener('message',({data:m})=>{if(m?.type==='frame')stats.frames++;});
    }
    postMessage(m,...rest) {if(m?.type==='batch')stats.batches++;return super.postMessage(m,...rest);}
  };
  const frame=()=>{stats.raf++;requestAnimationFrame(frame);};
  requestAnimationFrame(frame);
}
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function ready(page) {
  for(let i=0;i<150;i++) {
    if(await page.evaluate(()=>document.body.dataset.compute!==undefined && !!window.__pacing)) return true;
    await sleep(200);
  }
  return false;
}
// the counters over secs seconds -> per second
async function rate(page) {
  const a=await page.evaluate(()=>({...window.__pacing,t:performance.now()}));
  await sleep(secs*1000);
  const b=await page.evaluate(()=>({...window.__pacing,t:performance.now()}));
  const s=(b.t-a.t)/1000;
  return {batches:(b.batches-a.batches)/s, frames:(b.frames-a.frames)/s, raf:(b.raf-a.raf)/s};
}
async function rpc(page,method,params={}) {
  const r=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:1,method,params});
  if(!r || r.error) throw new Error(JSON.stringify(r));
  return r.result;
}
const f1=x=>x.toFixed(1);
let failed=0;
const idleCheck=(what,r)=>{
  if(maxIdle && r.batches>maxIdle) {failed++;console.log(`FAIL ${what}: ${f1(r.batches)} batches/s > ${maxIdle}`);}
};

try {
  for(const engine of engines) {
    const browser=engine==='chromium'
      ? await pw.chromium.launch({headless:true,...chromiumOptions()})
      : await launchWasm(pw[engine],{headless:true});
    try {
      console.log(`--- ${engine} ${browser.version()}${host?` (host: ${host})`:''}`);
      const open=async (url,width=1280,height=800)=>{
        const context=await browser.newContext({locale:'de-DE',viewport:{width,height}});
        await context.addInitScript(instrument);
        const page=await context.newPage(), lines=[];
        page.on('console',m=>lines.push(m.text()));
        page.on('pageerror',e=>lines.push(`pageerror ${e}`));
        await page.goto(`${origin}${url}`);
        if(!await ready(page)) throw new Error(`${url}: did not start`);
        return {context,page,lines};
      };
      // gamedemo at rest: paused until a click takes the pointer
      {
        const {context,page}=await open('/gamedemo/?agent=1&compute=simd');
        await sleep(3000);
        const r=await rate(page);
        console.log(`gamedemo at rest: ${f1(r.batches)} batches/s, ${f1(r.frames)} CPU frames/s (page ${f1(r.raf)} animation frames/s)`);
        idleCheck('gamedemo at rest',r);
        await context.close();
      }
      // gamedemo's view turning by itself on the CPU path (?gpu=0): every tick draws a frame the page shows
      {
        const {context,page}=await open(`/gamedemo/?compute=simd&gpu=0&gpuenv=jfgui_GPU_GAMEBENCH=${secs}`,640,400);
        await sleep(3000);
        const r=await rate(page);
        console.log(`gamedemo view without WebGPU, 640 x 400: ${f1(r.frames)} CPU frames/s, ${f1(r.batches)} batches/s (page ${f1(r.raf)} animation frames/s)`);
        await context.close();
      }
      // the same through WebGPU: the steps of the bench (the frames the app drew)
      if(engine==='firefox' && !(await browser.newPage().then(async p=>{const g=await p.evaluate(()=>!!navigator.gpu);await p.close();return g;}))) {
        console.log('gamedemo view through WebGPU: this browser has no WebGPU here');
      } else {
        const {context,page,lines}=await open(`/gamedemo/?compute=simd&gpu=1&gpuenv=jfgui_GPU_GAMEBENCH=${secs}`);
        // (the batches while the second step runs: the first RESULT ends the first step)
        let r=null;
        for(let i=0;i<Math.ceil(secs*4)+60 && !lines.some(t=>t.startsWith('path (a):'));i++) {
          if(!r && lines.some(t=>t.startsWith('RESULT '))) r=await rate(page); else await sleep(500);
        }
        const results=lines.filter(t=>t.startsWith('RESULT '));
        for(const t of results) {
          const v=k=>Number((new RegExp(`\\b${k}=([0-9.]+)`).exec(t)||[])[1]);
          const gap=(/\bdraw_gap=([0-9./]+)/.exec(t)||[])[1];
          console.log(`gamedemo view ${t.slice(7,t.indexOf(' presses='))}: ${f1(v('drawn')/v('secs'))} frames/s drawn (${v('drawn')} in ${v('secs')} s, gap med/p95/max ${gap} ms)`);
        }
        if(!results.length) {failed++;console.log(`FAIL gamedemo view: no RESULT (${lines.filter(t=>/error|GPU|path/i.test(t)).slice(0,3).join(' | ')})`);}
        if(r) console.log(`gamedemo view, second step: ${f1(r.batches)} batches/s (page ${f1(r.raf)} animation frames/s)`);
        await context.close();
      }
      // screentest at rest, then the caret blinking in its text box
      {
        const {context,page}=await open('/screentest/?agent=1&compute=simd');
        await sleep(3000);
        const r=await rate(page);
        console.log(`screentest at rest: ${f1(r.batches)} batches/s, ${f1(r.frames)} CPU frames/s`);
        idleCheck('screentest at rest',r);
        await rpc(page,'actions/execute',{action:'ui.focus',input:{id:101}});
        await sleep(1000);
        const c=await rate(page);
        console.log(`screentest, caret blinking: ${f1(c.batches)} batches/s, ${f1(c.frames)} CPU frames/s`);
        idleCheck('screentest, caret blinking',c);
        await context.close();
      }
    } finally {await browser.close();}
  }
} finally {await new Promise(r=>server.close(r));}
process.exit(failed?1:0);
