// End-to-end PaintCraft effect preview benchmarks using its ordinary UI, with saved
// output equality across scalar, SIMD128 and SIMD128 + four compute workers.
// SIZES=2048x2048,4096x3072 REPEATS=3 ENGINES=chromium,firefox node ... BUILD
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile,rename} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
import {execFileSync} from 'node:child_process';
import {cpus} from 'node:os';
const build=resolve(process.argv[2]||'build/wasm-compute/after'),artifact=resolve(build,'PaintCraft-compute');
await mkdir(artifact,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const sizes=(process.env.SIZES||'2048x2048,4096x3072').split(',').map(s=>s.split('x').map(Number));
const repeats=Number(process.env.REPEATS||3),results=[],references=new Map();
const effects=[{name:'Gaussian blur',command:472,slot:0,value:12},{name:'Motion blur',command:474,slot:1,value:32}];
if(process.env.RESUME==='1'){
  const previous=JSON.parse(await readFile(resolve(artifact,'results.json'),'utf8'));
  assert.equal(previous.repeats,repeats);assert.equal(previous.cpu,cpus()[0].model);
  for(const row of previous.results){
    assert.ok(row.ok);assert.equal(row.times.length,repeats);results.push(row);
    const key=`${row.width}x${row.height}:${row.effect}`;
    if(references.has(key))assert.equal(references.get(key),row.sha256);else references.set(key,row.sha256);
  }
  console.log(`Resuming ${results.length} completed cases`);
}
function compiling(){try{return process.platform==='win32'?/rustc\.exe/i.test(execFileSync('tasklist.exe',['/FI','IMAGENAME eq rustc.exe','/NH'],{encoding:'utf8'})):!!execFileSync('pgrep',['-x','rustc'],{encoding:'utf8'}).trim();}catch{return false;}}
async function idle(){let announced=false;while(compiling()){if(!announced){console.log('Waiting for compiler processes before measurement');announced=true;}await new Promise(r=>setTimeout(r,2000));}}
const server=createServer(async(req,res)=>{try{
  const url=new URL(req.url,'http://localhost');if(url.pathname==='/favicon.ico'){res.writeHead(204).end();return;}
  const path=resolve(build,'web','.'+decodeURIComponent(url.pathname.endsWith('/')?url.pathname+'index.html':url.pathname));
  if(!path.startsWith(resolve(build,'web')+sep))throw Error('path');
  let content=await readFile(path);
  // Slow scalar baselines may exceed the production agent deadline. Only this
  // isolated benchmark host extends it; the shipped loader remains unchanged.
  if(path.endsWith('loader.js'))content=Buffer.from(content.toString().replace('},30000);','},240000);'));
  res.writeHead(200,{'content-type':{'.js':'text/javascript','.wasm':'application/wasm','.html':'text/html','.css':'text/css','.json':'application/json'}[extname(path)]||'application/octet-stream','cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(content);
}catch(e){res.writeHead(404).end(String(e));}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
let id=0;
const value=(s,n)=>s.widgets.find(w=>w.id===n)?.value;
const label=(s,n)=>s.widgets.find(w=>w.id===n)?.label;
async function rpc(page,method,params={}){const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method,params});if(r.error)throw Error(JSON.stringify(r.error));return r.result;}
const state=page=>rpc(page,'state/inspect',{domain:'ui',limit:500});
async function action(page,name,input){for(let i=0;i<8;i++){const s=await state(page);try{return await rpc(page,'actions/execute',{action:name,input,expectedRevision:s.revision});}catch(e){if(!e.message.includes('revision_conflict')||i===7)throw e;}}}
async function until(page,predicate,description){const deadline=Date.now()+240000;let s;do{s=await state(page);if(predicate(s))return s;await new Promise(r=>setTimeout(r,30));}while(Date.now()<deadline);throw Error(`timeout ${description}: ${JSON.stringify(s)}`);}
async function click(page,match){const s=await state(page),w=s.widgets.slice().reverse().find(w=>w.layer>=s.modal&&(typeof match==='number'?w.id===match:w.label===match));assert.ok(w,`widget ${match}`);await action(page,'ui.click',{id:w.id,window:w.window});}
async function key(page,key){await page.locator('#input').focus();await page.keyboard.press(key);}
const command=(page,id)=>action(page,'ui.command',{id});
async function preview(page){return until(page,s=>value(s,9991)===49,'completed effect preview');}
function bitmap(w,h){
  const bytes=Buffer.alloc(54+w*h*4);bytes.write('BM');bytes.writeUInt32LE(bytes.length,2);bytes.writeUInt32LE(54,10);bytes.writeUInt32LE(40,14);bytes.writeInt32LE(w,18);bytes.writeInt32LE(h,22);bytes.writeUInt16LE(1,26);bytes.writeUInt16LE(32,28);
  let seed=0x12345678;
  for(let y=0;y<h;y++)for(let x=0;x<w;x++){
    seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;const i=54+(y*w+x)*4;
    bytes[i]=(x+y+(seed&31))&255;bytes[i+1]=((y>>2)+(seed&63))&255;bytes[i+2]=((x>>2)+(y>>3)+(seed&15))&255;bytes[i+3]=255;
  }
  return bytes;
}
async function savePixels(page,name){
  await command(page,103);await click(page,81932);await key(page,'Control+a');await page.keyboard.insertText('/'+name);await key(page,'Tab');await click(page,'Speichern');
  await until(page,s=>s.widgets.some(w=>w.label==='OK'),'PNG options');await click(page,'OK');
  await until(page,s=>value(s,9905)===0&&!s.widgets.some(w=>w.label==='Das Bild wird im Browserspeicher gesichert …'),'PNG durable');
  return page.evaluate(async name=>{
    const png=await new Promise((resolve,reject)=>{const r=indexedDB.open('jfgui-files-v1',1);r.onsuccess=()=>{const q=r.result.transaction('files','readonly').objectStore('files').get('PaintCraft:/'+name);q.onsuccess=()=>resolve(q.result.bytes);q.onerror=()=>reject(q.error);};r.onerror=()=>reject(r.error);});
    if(png.byteLength<=100)throw Error('PNG output missing');
    return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',png)),b=>b.toString(16).padStart(2,'0')).join('');
  },name);
}
async function record(){await writeFile(resolve(artifact,'results.pending.json'),JSON.stringify({cpu:cpus()[0].model,logicalCpus:cpus().length,repeats,method:'Effect command to complete preview, one warm-up; PNG equality after commit; milliseconds',results},null,2));await rename(resolve(artifact,'results.pending.json'),resolve(artifact,'results.json'));}
try{
  for(const engine of (process.env.ENGINES||'chromium,firefox').split(',')){
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try{
      for(const row of results.filter(r=>r.engine===engine))assert.equal(row.browser,browser.version());
      for(const [width,height]of sizes){const bmp=bitmap(width,height);
      for(const mode of (process.env.MODES||'scalar,simd,threads').split(',')){
        const pending=effects.filter(effect=>!results.some(r=>r.engine===engine&&r.mode===mode&&r.width===width&&r.height===height&&r.effect===effect.name));
        if(!pending.length)continue;
        await idle();
        const context=await browser.newContext({viewport:{width:1280,height:800}}),page=await context.newPage(),errors=[];
        await page.addInitScript(()=>{addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        page.on('pageerror',e=>{errors.push(e.message);console.error(e.message);});page.on('console',m=>{if(m.type()==='error'&&!m.text().includes('404')){errors.push(m.text());console.error(m.text());}});
        await page.goto(`http://127.0.0.1:${server.address().port}/PaintCraft/?agent=1&theme=light&compute=${mode}&workers=4`);
        await page.waitForFunction(()=>window.__jfguiReady,null,{timeout:90000});
        const compute=await page.evaluate(()=>window.__jfguiCompute);assert.equal(compute.mode,mode);
        await command(page,105);await page.locator('#files').setInputFiles({name:'large.bmp',mimeType:'image/bmp',buffer:bmp});
        await until(page,s=>value(s,9913)===width&&value(s,9914)===height,'large image imported');
        for(const effect of pending){
          console.log(`Warm-up ${engine} ${mode} ${width}x${height} ${effect.name}`);
          await command(page,effect.command);await preview(page);
          await click(page,0x60000000|((0x1b68+effect.slot*16)<<4)|1);
          await key(page,'Control+a');await page.keyboard.insertText(String(effect.value));await page.keyboard.press('Enter');
          await new Promise(r=>setTimeout(r,100));await preview(page);await click(page,'OK');await until(page,s=>value(s,9991)===0,'warm-up committed');
          await key(page,'Control+z');await until(page,s=>value(s,9991)===0,'warm-up undone');
          const times=[];let sha256;
          for(let iteration=0;iteration<repeats;iteration++){
            await idle();
            const start=performance.now();await command(page,effect.command);await preview(page);const elapsed=performance.now()-start,interference=compiling();
            if(!interference)times.push(elapsed);
            await click(page,'OK');await until(page,s=>value(s,9991)===0,'effect committed');
            if(iteration===0&&!interference)sha256=await savePixels(page,`result-${width}-${height}-${effect.command}.png`);
            await key(page,'Control+z');await until(page,s=>value(s,9991)===0,'undo');
            if(interference){console.log(`Discarded sample ${iteration+1} overlapping compilation`);iteration--;}
          }
          const keyRef=`${width}x${height}:${effect.name}`;
          if(references.has(keyRef))assert.equal(sha256,references.get(keyRef),'saved image bytes must match across backends/browsers');else references.set(keyRef,sha256);
          assert.deepEqual(errors,[]);
          const medianMs=[...times].sort((a,b)=>a-b)[Math.floor(times.length/2)];
          results.push({engine,browser:browser.version(),mode,compute,width,height,effect:effect.name,parameter:effect.value,times,medianMs,sha256,ok:true});
          console.log(JSON.stringify(results.at(-1)));await record();
        }
        await page.screenshot({path:resolve(artifact,`${engine}-${mode}-${width}x${height}.png`)});await context.close();
      }
    }}finally{await browser.close();}
  }
}finally{server.close();await record();}
