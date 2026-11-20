// (G7b part 2) the demo in the browser: the page's WebGPU device draws the view, lib/ui's HUD over it (veils), the
// beacon loaded through Assets (Loading::Frames there); then the measurement of path (a) in the browser (canvas,
// requestAnimationFrame, the UI in its worker): ?gpu=1&gpuenv=jfgui_GPU_GAMEBENCH=SECS runs the pacing steps (no HUD,
// veils, read-back), SECS seconds each; the page prints a RESULT line per step (bench.rs, probe.rs: the frames the app
// drew; nothing reads the screen in a page).  A picture of the running page goes to build/gamedemo_web.png.
//   python build_rs.py app gamedemo --target web
//   node apps/gamedemo/tests/bench_web.mjs [build] [--secs=3] [--mode=simd] [--headed]
// Playwright: tools/tests/playwright.mjs (PLAYWRIGHT_MODULE).
import {createServer} from 'node:http';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright(), dir=resolve(build,'web','gamedemo');

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
let failed=0;
try {
  const browser=await pw.chromium.launch({headless:!args.includes('--headed'),...chromiumOptions()});
  try {
    console.log(`--- chromium ${browser.version()}`);
    const mode=option('mode','simd');
    // the demo itself: the view, the HUD, the beacon (the status bar says "Modell bereit")
    {
      const context=await browser.newContext({locale:'de-DE',viewport:{width:1024,height:640}});
      const page=await context.newPage(), errors=[];
      page.on('pageerror',e=>errors.push(String(e)));
      await page.goto(`${origin}/index.html?gpu=1&compute=${mode}`);
      let ready=false;
      for(let i=0;i<100&&!ready;i++) {
        await new Promise(r=>setTimeout(r,200));
        ready=await page.evaluate(()=>!!window.jfguiAgent);
      }
      await new Promise(r=>setTimeout(r,4000));
      const shot=await page.screenshot();
      await writeFile(resolve(build,'gamedemo_web.png'),shot);
      console.log(`${errors.length?'FAIL':'PASS'} the demo ran ${errors.length?errors.join(' | '):'without page errors'} (picture: build/gamedemo_web.png)`);
      if(errors.length) failed++;
      await context.close();
    }
    // the measurement
    const context=await browser.newContext({locale:'de-DE',viewport:{width:1280,height:800}});
    const page=await context.newPage(), lines=[];
    let done;
    const finished=new Promise(r=>done=r);
    page.on('console',m=>{
      const t=m.text();
      if(t.startsWith('RESULT ')) {lines.push(t);console.log(t);}
      if(t.startsWith('path (a):')) {console.log(t);done();}
    });
    await page.goto(`${origin}/index.html?gpu=1&compute=${mode}&gpuenv=jfgui_GPU_GAMEBENCH=${option('secs','3')}`);
    await Promise.race([finished,new Promise(r=>setTimeout(r,120000))]);
    if(lines.length<3) {console.log(`FAIL ${lines.length} of 3 steps measured`);failed++;}
    await context.close();
  } finally {await browser.close();}
} finally {await new Promise(r=>server.close(r));}
process.exit(failed?1:0);
