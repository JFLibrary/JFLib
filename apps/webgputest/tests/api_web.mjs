// lib/gpu's WebGPU backend behind its API (apps/webgputest/main.rs) in each of the three bundles: the
// program says ok / FAIL per check on the console, this test reads them.  In a browser without a working
// WebGPU it says SKIP (Playwright's Firefox here).  Playwright: tools/tests/playwright.mjs.
//   python build_rs.py app webgputest --target web
//   node apps/webgputest/tests/api_web.mjs [build] [--engines=chromium,firefox] [--modes=simd,threads,scalar]
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright(), dir=resolve(build,'web','webgputest');
const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(parts.length ? parts : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const body=await readFile(file);
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(body);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
let failures=0, ran=0;
try {
  for(const engine of option('engines','chromium,firefox').split(',').filter(Boolean)) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const mode of option('modes','simd,threads,scalar').split(',').filter(Boolean)) {
        const page=await browser.newPage(), lines=[];
        const finished=new Promise(ok=>page.on('console',m=>{
          const t=m.text().trimEnd();
          lines.push(t);
          if(/^webgputest: |^SKIP webgputest/.test(t)) ok(t);
        }));
        page.on('pageerror',e=>lines.push(`pageerror ${e}`));
        await page.goto(`http://127.0.0.1:${server.address().port}/index.html?compute=${mode}`);
        const end=await Promise.race([finished,new Promise(ok=>setTimeout(()=>ok('timeout'),60000))]);
        const fail=lines.filter(l=>l.startsWith('FAIL')||l.startsWith('pageerror'));
        const count=lines.filter(l=>l.startsWith('ok ')).length;
        const compute=await page.evaluate(()=>window.__jfguiCompute?.mode);
        for(const l of fail) console.log(`${engine} ${mode}: ${l}`);
        if(args.includes('--verbose')) for(const l of lines) console.log(l);
        if(end.startsWith('SKIP')) console.log(`SKIP ${engine} ${mode}: ${end}`);
        else {
          ran++;
          const good=end.startsWith('webgputest: ')&&/ 0 failed/.test(end)&&!fail.length&&count>0&&compute===mode;
          if(!good) failures++;
          console.log(`${good?'PASS':'FAIL'} ${engine} ${browser.version()} ${compute}: ${end} (${lines.find(l=>l.startsWith('adapter:'))||'no adapter line'})`);
        }
        await page.close();
      }
    } finally {await browser.close();}
  }
} finally {await new Promise(r=>server.close(r));}
if(!ran) console.log('SKIP: no browser here has a working WebGPU');
if(failures) process.exitCode=1;
