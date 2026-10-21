// Actual browser SIMD/atomic-memory/worker acceptance, including nonisolated fallback.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from './playwright.mjs';
const build=resolve(process.argv[2]||'build/wasm-compute/after');
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const results=[];
const server=createServer(async(req,res)=>{try {
  const url=new URL(req.url,'http://localhost'), parts=decodeURIComponent(url.pathname).split('/').filter(Boolean);
  const isolated=parts.shift()!=='plain';
  if(parts.length===1)parts.push('index.html');
  const path=resolve(build,'web',...parts);
  if(!path.startsWith(resolve(build,'web')+sep))throw Error('path');
  const content=await readFile(path);
  res.writeHead(200,{'content-type':{'.js':'text/javascript','.wasm':'application/wasm','.html':'text/html','.css':'text/css','.json':'application/json'}[extname(path)]||'application/octet-stream','cache-control':'no-store',...(isolated?{'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}:{})}).end(content);
}catch(e){res.writeHead(404).end(String(e));}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
try {
  for(const engine of (process.env.ENGINES||'chromium,firefox').split(',')) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const [app,mode,isolated] of [['portabletest','threads',true],['PaintCraft','threads',true],['PaintCraft','simd',true],['PaintCraft','scalar',true],['PaintCraft','auto',false]]) {
        const page=await browser.newPage({viewport:{width:1280,height:800}}),logs=[],errors=[];
        await page.addInitScript(()=>{addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        if(app==='portabletest')await page.addInitScript(()=>{const click=HTMLInputElement.prototype.click;HTMLInputElement.prototype.click=function(){if(this.type!=='file')click.call(this);};});
        page.on('console',m=>{if(m.type()==='error'&&!m.text().includes('404')){errors.push(m.text());console.log(engine,app,m.text());}else if(!m.text().includes('404'))logs.push(m.text());});
        page.on('pageerror',e=>{errors.push(e.message);console.log(engine,app,e.message);});
        await page.goto(`http://127.0.0.1:${server.address().port}/${isolated?'isolated':'plain'}/${app}/?compute=${mode}&workers=4`);
        try {await page.waitForFunction(()=>window.__jfguiReady,null,{timeout:90000});}
        catch(e) {throw Error(`${engine} ${app}: ${e.message}; logs=${logs}; errors=${errors}; status=${await page.locator('#status').textContent()}`);}
        const compute=await page.evaluate(()=>window.__jfguiCompute);
        assert.equal(compute.mode,mode==='auto'?'simd':mode);assert.deepEqual(errors,[]);
        if(app==='portabletest') {
          assert.ok(logs.some(x=>x.includes('overlap=4 nested=320 cancellation restart')),logs.join('\n'));
          assert.ok(logs.some(x=>x.includes('digest=5a22a1f803872742')),logs.join('\n'));
        }
        if(app==='PaintCraft'&&mode==='threads')assert.equal(compute.workers,4);
        results.push({engine,app,mode,isolated,compute,logs,ok:true});console.log(JSON.stringify(results.at(-1)));
        await page.close();
      }
    }finally{await browser.close();}
  }
}finally {
  server.close();await mkdir(resolve(build,'compute-tests'),{recursive:true});
  await writeFile(resolve(build,'compute-tests/results.json'),JSON.stringify(results,null,2));
}
