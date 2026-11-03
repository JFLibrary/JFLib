// What the output through WebGPU costs in the browser against the old path (docs/GPU.md "Ausgabe im
// Browser"): the real bundles in a browser of its own (Chrome, headless with the real GPU; a profile in a
// temp folder), ?gpu=0 against ?gpu=1 in turn, one page per round.  Per scenario: the UI worker's time per
// batch (a wrapper around the worker's onmessage, served with worker.js), the page's main thread per frame
// (CDP Performance.getMetrics TaskDuration; includes this script's small evaluate calls), late frames (a
// display frame that found the last batch unfinished: new input had to wait), and over a quiet time the CPU
// of the browser's processes, its memory and what moved.  What this does not see: the real display (headless
// Chrome composes on demand: its trace has no swap for most frames, "to the screen" takes a real window), the
// GPU's time (timestamps are not built).
//   python build_rs.py app gallery androidGallery --target web --release
//   node tools/tests/web_gpu_bench.mjs build/release [--rounds=3] [--mode=simd] [--size=1920x1000] [--variants=cpu,gpu] [--idle=10]
import {createServer} from 'node:http';
import {readFile, mkdtemp, rm} from 'node:fs/promises';
import {execFileSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from './playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build/release');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const rounds=+option('rounds','3'), mode=option('mode','simd'), idleSeconds=+option('idle','10');
const [width,height]=option('size','1920x1000').split('x').map(Number);
const variants=option('variants','cpu,gpu').split(',');
const a11y=option('a11y','1'); // 0: without the accessibility mirror in the page (?a11y=0)
const scenarios=option('scenarios','full,small,scroll,pan,idle').split(',');
const pw=loadPlaywright();

const WRAP=`
{const inner=self.onmessage;let list=[];
self.onmessage=e=>{
  if(e.data.type==="bench-read"){postMessage({type:"bench",list,ops:self.__ops});list=[];if(self.__ops)for(const k in self.__ops)delete self.__ops[k];return;}
  const t=performance.now(), r=inner(e), d=performance.now()-t;
  if(e.data.type==="batch") list.push(d);
  return r;
};}
`;
const OPS=`
{const inner=GpuHost.prototype.dispatch;const s=self.__ops={};
GpuHost.prototype.dispatch=function(op,a,n){const t=performance.now();const r=inner.call(this,op,a,n);const d=performance.now()-t;const e=s[op]||(s[op]={n:0,ms:0,top:0});e.n++;e.ms+=d;if(d>e.top)e.top=d;return r;};}
`;
const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const root=resolve(build,'web');
    const file=resolve(root,...parts);
    if(!file.startsWith(root+sep)) throw new Error('outside');
    let body=await readFile(file);
    if(parts.at(-1)==='worker.js') body=Buffer.concat([body,Buffer.from(WRAP)]);
    // (--ops: which calls of gpu.js the time goes to)
    if(parts.at(-1)==='gpu.js'&&args.includes('--ops')) body=Buffer.concat([body,Buffer.from(OPS)]);
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(body);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin=`http://127.0.0.1:${server.address().port}`;

function instruments() {
  const b=window.__bench={flight:0,frames:0,bytes:0,frameMs:0};
  const Base=window.Worker;
  window.Worker=class extends Base {
    constructor(...a) {
      super(...a);
      window.__worker=this;
      this.addEventListener('message',({data:m})=>{
        if(m.type==='batch-complete') b.flight--;
        if(m.type==='frame') {b.frames++;for(const p of m.patches||[]) b.bytes+=p.width*p.height*4;}
        if(m.type==='bench'&&window.__benchReply) {window.__lastOps=m.ops;window.__benchReply(m.list);}
      });
    }
    postMessage(m,...rest) {if(m.type==='batch') b.flight++;super.postMessage(m,...rest);}
  };
  window.__ready=false;addEventListener('jfgui-ready',()=>window.__ready=true);
}
const counters=async page=>{
  const text=await page.evaluate(()=>window.jfguiAgent.gpuTest(0));
  return Object.fromEntries(text.split(' ').map(p=>{const k=p.indexOf('=');return [p.slice(0,k),isNaN(+p.slice(k+1))||p.slice(k+1)===''?p.slice(k+1):+p.slice(k+1)];}));
};
const workerTimes=page=>page.evaluate(()=>new Promise(r=>{window.__benchReply=r;window.__worker.postMessage({type:"bench-read"});}));
const raf=page=>page.evaluate(()=>new Promise(requestAnimationFrame));
const median=a=>{if(!a.length)return 0;const s=[...a].sort((x,y)=>x-y);return s[s.length>>1];};
const pct=(a,q)=>{if(!a.length)return 0;const s=[...a].sort((x,y)=>x-y);return s[Math.min(s.length-1,Math.floor(s.length*q))];};
async function taskDuration(cdp) {
  const {metrics}=await cdp.send('Performance.getMetrics');
  return metrics.find(m=>m.name==='TaskDuration').value*1000;
}
// the scenario: step(i) for n display frames -> what it cost
async function scenario(page,cdp,n,step) {
  await workerTimes(page);
  const before=await page.evaluate(()=>({...window.__bench}));
  const c0=await counters(page);
  const t0=await taskDuration(cdp);
  let late=0;
  const w0=performance.now();
  for(let i=0;i<n;i++) {
    await step(i);
    await raf(page);
    if(await page.evaluate(()=>window.__bench.flight>0)) late++;
  }
  const wall=performance.now()-w0;
  await page.evaluate(()=>new Promise(r=>setTimeout(r,100)));
  const main=await taskDuration(cdp)-t0;
  const list=await workerTimes(page);
  const after=await page.evaluate(()=>({...window.__bench}));
  const c1=await counters(page);
  const ops=args.includes('--ops')?await page.evaluate(()=>window.__lastOps):undefined;
  return {ops,n,workerMedian:+median(list).toFixed(2),workerP95:+pct(list,.95).toFixed(2),workerMax:+Math.max(0,...list).toFixed(1),batches:list.length,
    mainPerFrame:+(main/n).toFixed(2),late,msPerStep:+(wall/n).toFixed(1),
    framesCpu:after.frames-before.frames,bytesPage:after.bytes-before.bytes,framesGpu:c1.frames-c0.frames,uploaded:c1.upload_bytes-c0.upload_bytes,base:c1.base_px-c0.base_px,tiles:c1.canvas_tiles,splitFrames:c1.split_frames-c0.split_frames};
}
// the browser's processes (their command lines name this profile): cpu seconds, memory
function processes(profile) {
  const script=`Get-CimInstance Win32_Process -Filter "Name='chrome.exe'" | Where-Object {$_.CommandLine -like '*${profile.replace(/\\/g,'\\\\').split(/[\\/]/).pop()}*'} | ForEach-Object {$p=Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue; if($p){ $k=if($_.CommandLine -match '--type=([a-z-]+)'){$Matches[1]}else{'browser'}; if($_.CommandLine -match 'utility-sub-type=([a-z.]+)'){$k=$k+':'+$Matches[1]}; '{0} {1} {2} {3} {4}' -f $k,$_.ProcessId,$p.CPU,$p.WorkingSet64,$p.PrivateMemorySize64 } }`;
  const out=execFileSync('powershell.exe',['-NoProfile','-NonInteractive','-Command',script],{encoding:'utf8'});
  return out.split(/\r?\n/).filter(Boolean).map(l=>{const [kind,pid,cpu,ws,priv]=l.split(' ');return {kind,pid:+pid,cpu:+cpu.replace(',','.'),ws:+ws,priv:+priv};});
}
const sum=(list,f)=>list.reduce((n,x)=>n+f(x),0);

async function round(variant,app,which) {
  const profile=await mkdtemp(join(tmpdir(),'jfgui-g4d-'));
  const context=await pw.chromium.launchPersistentContext(profile,{headless:true,viewport:{width,height:height+38},locale:'de-DE',...chromiumOptions()});
  const out={variant,app};
  try {
    const page=await context.newPage();
    await page.addInitScript(instruments);
    const cdp=await context.newCDPSession(page);
    await cdp.send('Performance.enable');
    const url=`${origin}/${app}/index.html?agent=1&gpu=${variant==='gpu'?1:0}&theme=light&compute=${mode}&a11y=${a11y}`;
    const t0=Date.now();
    await page.goto(url);
    await page.waitForFunction(()=>window.__ready,null,{timeout:60000});
    out.toFirstFrame=Date.now()-t0;
    if(variant==='gpu') {
      const end=Date.now()+15000;
      let c=await counters(page);
      while(c.on!==1&&Date.now()<end) {await new Promise(r=>setTimeout(r,10));c=await counters(page);}
      if(c.on!==1) throw new Error(`no GPU output: ${JSON.stringify(c)}`);
      out.toSwitch=Date.now()-t0;
      out.adapter=c.adapter;
    } else {
      await new Promise(r=>setTimeout(r,300));
    }
    await page.waitForTimeout(500);
    const box=await page.locator('#screen').boundingBox();
    const centre=[box.x+box.width/2,box.y+box.height/2];
    if(which.includes('full')) out.full=await scenario(page,cdp,60,async i=>{await page.evaluate(()=>window.jfguiAgent.gpuTest(3));});
    // hover from one button to another: a small change
    if(which.includes('small')) {
      const {result}=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:3,method:'state/inspect',params:{domain:'ui',limit:800}});
      const buttons=result.widgets.filter(w=>w.kind===2&&w.layer===0&&w.rect[1]>40&&w.rect[1]+w.rect[3]<box.height&&w.rect[2]>20);
      const [a,b]=[buttons[0],buttons[buttons.length>>1]];
      if(!a||a===b) throw new Error('no two buttons to hover');
      const at=w=>[box.x+w.rect[0]+w.rect[2]/2,box.y+w.rect[1]+w.rect[3]/2];
      out.small=await scenario(page,cdp,120,async i=>{await page.mouse.move(...at(i%2?a:b));});
    }
    // the wheel, one step per display frame
    if(which.includes('scroll')) {
      await page.mouse.move(...centre);
      out.scroll=await scenario(page,cdp,120,async i=>{await page.mouse.wheel(0,(i%60)<30?60:-60);});
    }
    // a canvas: go to "Bild & Leinwand", drag the image view
    if(which.includes('pan')) {
      const clickOn=async label=>{
        const inspect=await page.evaluate(q=>window.jfguiAgent.request(q),{jsonrpc:'2.0',id:1,method:'state/inspect',params:{domain:'ui',limit:800}});
        const w=inspect.result.widgets.filter(x=>x.label===label).sort((a,b)=>a.layer-b.layer).at(-1);
        if(w) await page.mouse.click(box.x+w.rect[0]+w.rect[2]/2,box.y+w.rect[1]+w.rect[3]/2);
        await page.waitForTimeout(300);
        return !!w;
      };
      if(await clickOn('Seiten')&&await clickOn('Bild & Leinwand')) {
        await page.waitForTimeout(800);
        await page.mouse.move(box.x+box.width/2,box.y+260);
        await page.mouse.down();
        out.pan=await scenario(page,cdp,120,async i=>{const d=(i%60)<30?(i%60)*4:(60-(i%60))*4;await page.mouse.move(box.x+box.width/2+d,box.y+260+d/2);});
        await page.mouse.up();
        await page.waitForTimeout(400);
      } else out.pan=null;
    }
    if(which.includes('idle')) {
      await page.mouse.move(box.x+box.width/2,box.y+box.height-4);
      await page.waitForTimeout(1500);
      await workerTimes(page);
      const c0=await counters(page), f0=await page.evaluate(()=>window.__bench.frames), p0=processes(profile);
      const t=performance.now();
      await page.waitForTimeout(idleSeconds*1000);
      const seconds=(performance.now()-t)/1000;
      const p1=processes(profile), c1=await counters(page), f1=await page.evaluate(()=>window.__bench.frames), list=await workerTimes(page);
      const per=k=>+(sum(p1.filter(x=>x.kind.startsWith(k)),x=>x.cpu)-sum(p0.filter(x=>x.kind.startsWith(k)),x=>x.cpu)).toFixed(3);
      out.idle={seconds:+seconds.toFixed(1),framesCpu:f1-f0,framesGpu:c1.frames-c0.frames,ticks:list.length,workerMsPerTick:+median(list).toFixed(3),
        cpuRenderer:per('renderer'),cpuGpu:per('gpu-process'),cpuBrowser:per('browser'),cpuAll:+(sum(p1,x=>x.cpu)-sum(p0,x=>x.cpu)).toFixed(3)};
      const mb=v=>+(v/1048576).toFixed(1);
      const part=k=>{const l=p1.filter(x=>x.kind.startsWith(k));return {ws:mb(sum(l,x=>x.ws)),priv:mb(sum(l,x=>x.priv))};};
      out.memory={renderer:part('renderer'),gpu:part('gpu-process'),all:{ws:mb(sum(p1,x=>x.ws)),priv:mb(sum(p1,x=>x.priv))},reserved:mb(c1.reserved),
        ua:await page.evaluate(async()=>{try{const m=await performance.measureUserAgentSpecificMemory();const by={};for(const b of m.breakdown)for(const t of b.types)by[t]=(by[t]||0)+b.bytes;return {bytes:m.bytes,by};}catch(e){return String(e);}})};
    }
    await page.close();
  } finally {
    await context.close();
    await rm(profile,{recursive:true,force:true}).catch(()=>{});
  }
  return out;
}
const results=[];
try {
  for(let r=0;r<rounds;r++) {
    for(const variant of r%2?[...variants].reverse():variants) {
      for(const [app,which] of [['gallery',scenarios.filter(s=>s!=='pan')],['androidGallery',scenarios.filter(s=>s==='pan')]]) {
        if(!which.length) continue;
        const res=await round(variant,app,which);
        res.round=r;
        results.push(res);
        console.log(JSON.stringify(res));
      }
    }
  }
} finally {await new Promise(r=>server.close(r));}
