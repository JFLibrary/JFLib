// Actual Chromium/Firefox checks for canvas resets, transfer volume and input backlog.
// node tools/tests/web_responsiveness.mjs build/wasm-responsive/after [baseline-web]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from './playwright.mjs';
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const build=resolve(process.argv[2]||'build/wasm-responsive/after');
const roots={after:resolve(build,'web')};if(process.argv[3])roots.before=resolve(process.argv[3]);
const artifact=resolve(build,'responsiveness');await mkdir(artifact,{recursive:true});
const server=createServer(async(req,res)=>{try{
  const parts=decodeURIComponent(new URL(req.url,'http://localhost').pathname).split('/').filter(Boolean);
  if(parts.length===2)parts.push('index.html');
  const root=roots[parts.shift()];if(!root)throw Error('unknown build');
  const file=resolve(root,...parts);if(!file.startsWith(root+sep))throw Error('invalid path');
  res.writeHead(200,{'content-type':{'.html':'text/html','.js':'text/javascript','.css':'text/css','.wasm':'application/wasm'}[extname(file)]||'application/octet-stream','cache-control':'no-store'}).end(await readFile(file));
}catch(error){res.writeHead(404).end(String(error));}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`,results=[];
async function rpc(page,method,params={}){
  const result=await page.evaluate(({method,params})=>window.jfguiAgent.request({jsonrpc:'2.0',id:1,method,params}),{method,params});
  assert.ok(result&&!result.error,JSON.stringify(result));return result.result;
}
async function settle(page){await rpc(page,'agent/info');await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));}
function instruments(){
  const stats=window.__renderStats={frames:0,bytes:0,resets:0,sameSizeResets:0,batches:0,flight:0,maxFlight:0,delay:0};
  for(const key of ['width','height']){
    const descriptor=Object.getOwnPropertyDescriptor(HTMLCanvasElement.prototype,key);
    Object.defineProperty(HTMLCanvasElement.prototype,key,{...descriptor,set(value){
      if(this.id==='screen'){stats.resets++;if(descriptor.get.call(this)===value)stats.sameSizeResets++;}
      descriptor.set.call(this,value);
    }});
  }
  const WorkerBase=window.Worker;
  window.Worker=class extends WorkerBase{
    constructor(...args){super(...args);this.addEventListener('message',({data:m})=>{
      if(m.type==='frame'){stats.frames++;stats.bytes+=m.buffer?m.patches.reduce((n,p)=>n+p.width*p.height*4,0):(m.patches||[{pixels:m.pixels}]).reduce((n,p)=>n+p.pixels.byteLength,0);}
      if(m.type==='batch-complete')stats.flight--;
    });}
    postMessage(message,...rest){
      if(message.type==='batch'){
        stats.batches++;stats.flight++;stats.maxFlight=Math.max(stats.maxFlight,stats.flight);
        if(stats.delay){setTimeout(()=>super.postMessage(message,...rest),stats.delay);return;}
      }
      super.postMessage(message,...rest);
    }
  };
  window.__ready=false;addEventListener('jfgui-ready',()=>window.__ready=true);
}
async function burst(page,count){return page.evaluate(async count=>{
  const canvas=document.querySelector('#screen'),r=canvas.getBoundingClientRect(),before={...window.__renderStats},start=performance.now();
  for(let i=0;i<count;i++)canvas.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,isPrimary:true,pointerId:1,pointerType:'mouse',buttons:0,clientX:r.left+Math.min(r.width-20,380+(i%30)*8),clientY:r.top+Math.min(r.height-20,350)}));
  const reply=await window.jfguiAgent.request({jsonrpc:'2.0',id:77,method:'agent/info'});
  if(reply.error)throw Error(JSON.stringify(reply.error));
  const elapsedMs=performance.now()-start;
  await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
  const stats=window.__renderStats;
  return {count,elapsedMs,frames:stats.frames-before.frames,bytes:stats.bytes-before.bytes,resets:stats.resets-before.resets,sameSizeResets:stats.sameSizeResets-before.sameSizeResets,batches:stats.batches-before.batches,maxFlight:stats.maxFlight};
},count);}
try{
  for(const engine of ['chromium','firefox']){
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try{
      for(const variant of Object.keys(roots).reverse())for(const app of ['template','gallery','Kritzel','PaintCraft','jfWord']){
        const context=await browser.newContext({viewport:{width:1280,height:838}}),page=await context.newPage(),errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        await page.addInitScript(instruments);
        try{
          await page.goto(`${base}/${variant}/${app}/?agent=1&theme=light`);await page.waitForFunction(()=>window.__ready,{timeout:45000});await settle(page);
          const measure=await burst(page,120);
          if(variant==='after'){
            assert.equal(measure.sameSizeResets,0,'drawing never resets an unchanged canvas dimension');
            assert.equal(measure.resets,0,'hover never resizes the canvas');
            assert.equal(measure.maxFlight,1,'at most one worker batch in flight');
            assert.ok(measure.frames<=4,`hover burst rendered ${measure.frames} intermediate frames`);
            assert.ok(measure.elapsedMs<1000,`input backlog: ${measure.elapsedMs} ms`);
            // Slow-worker regression: new input waits in the host where stale hover can
            // be replaced; it cannot accumulate independent worker rendering requests.
            await page.evaluate(()=>{window.__renderStats.delay=80;});
            const slow=await burst(page,120);
            assert.equal(slow.maxFlight,1);assert.ok(slow.frames<=4);assert.ok(slow.elapsedMs<700);
            await page.evaluate(()=>{window.__renderStats.delay=0;});
            // Close a real modal before the resize; partial presentation must not keep it.
            if(app==='PaintCraft'||app==='Kritzel'){
              await page.locator('#input').press('Control+n');await settle(page);
              await page.locator('#input').press('Escape');await settle(page);
            }
            await page.setViewportSize({width:1001,height:751});
            await page.waitForFunction(()=>document.querySelector('#screen').width===1001&&document.querySelector('#screen').height===713);
            await settle(page);
            assert.equal(await page.evaluate(()=>window.__renderStats.sameSizeResets),0,'initialization and resize assign only changed dimensions');
            await page.screenshot({path:resolve(artifact,`${engine}-${app}.png`)});
          }
          assert.deepEqual(errors,[]);
          const result={engine,variant,app,ok:true,...measure};results.push(result);console.log(JSON.stringify(result));
        }catch(error){results.push({engine,variant,app,ok:false,error:String(error.stack||error),errors});console.error(`${engine} ${variant} ${app}: ${error.stack||error}`);await page.screenshot({path:resolve(artifact,`${engine}-${variant}-${app}-failure.png`)}).catch(()=>{});}
        finally{await context.close();}
      }
      for(const app of ['Kritzel','PaintCraft']){
        const context=await browser.newContext({viewport:{width:640,height:838},deviceScaleFactor:2}),page=await context.newPage();
        await page.addInitScript(instruments);
        try{
          await page.goto(`${base}/after/${app}/?agent=1`);await page.waitForFunction(()=>window.__ready);await settle(page);
          const measure=await burst(page,120);
          assert.equal(measure.resets,0);assert.equal(measure.sameSizeResets,0);assert.equal(measure.maxFlight,1);assert.ok(measure.elapsedMs<1000);
          results.push({engine,variant:'after',app,dpi:200,ok:true,...measure});
          await page.screenshot({path:resolve(artifact,`${engine}-${app}-dpi200.png`)});
        }finally{await context.close();}
      }
    }finally{await browser.close();}
  }
}finally{await writeFile(resolve(artifact,'results.json'),JSON.stringify(results,null,2));await new Promise(r=>server.close(r));}
if(results.some(r=>!r.ok))process.exitCode=1;
