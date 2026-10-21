// Real PaintCraft browser regressions. Run from RustLib; profiles and files are disposable.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, mkdir, writeFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const root=resolve('.'), build=resolve(process.argv[2]||'build/PaintCraft-security/after');
const deeper=process.argv.includes('--deeper');
const artifact=resolve(build,deeper?'deeper-browser':'security-browser'); await mkdir(artifact,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve(root,'build/browser-engines');
const pw=loadPlaywright();
const server=createServer(async(req,res)=>{try{
  const pathname=decodeURIComponent(new URL(req.url,'http://localhost').pathname);
  const filename=resolve(build,'web','.'+(pathname.endsWith('/')?pathname+'index.html':pathname));
  if(!filename.startsWith(resolve(build,'web')+sep))throw Error('path');
  const bytes=await readFile(filename);
  res.writeHead(200,{'content-type':{'.js':'text/javascript','.html':'text/html','.wasm':'application/wasm','.css':'text/css'}[extname(filename)]||'application/octet-stream','cache-control':'no-store',...(process.argv.includes('--isolated')?{'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}:{})}).end(bytes);
}catch(e){res.writeHead(404).end(String(e));}});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const url=`http://127.0.0.1:${server.address().port}/PaintCraft/`;
let id=0;const results=[];
async function rpc(page,method,params={}){
  const r=await page.evaluate(r=>window.jfguiAgent.request(r),{jsonrpc:'2.0',id:++id,method,params});
  if(r.error){const error=new Error(`${method}: ${JSON.stringify(r.error)}`);error.code=r.error.data?.code;throw error;}return r.result;
}
async function state(page){return rpc(page,'state/inspect',{domain:'ui',limit:500});}
async function action(page,action,input={}){for(let n=0;n<8;n++){const s=await state(page);try{return await rpc(page,'actions/execute',{action,input,expectedRevision:s.revision});}catch(e){if(e.code!=='revision_conflict'||n===7)throw e;}}}
async function until(fn,predicate,name){let s;for(let n=0;n<120;n++){s=await fn();if(predicate(s))return s;await new Promise(r=>setTimeout(r,100));}throw Error(`timeout ${name}: ${JSON.stringify({title:s?.title,modal:s?.modal,widgets:s?.widgets?.filter(w=>[9901,9903,9905,9918,1640].includes(w.id))})}`);}
const value=(s,id)=>s.widgets.find(w=>w.id===id)?.value;
const hasLabel=(s,label)=>s.widgets.some(w=>w.label===label);
const isSaving=s=>hasLabel(s,'Das Bild wird im Browserspeicher gesichert …');
async function click(page,match){const s=await state(page),w=s.widgets.slice().reverse().find(w=>w.layer>=s.modal&&(typeof match==='number'?w.id===match:w.label===match));assert.ok(w,`widget ${match}`);return action(page,'ui.click',{id:w.id,window:w.window});}
async function key(page,key){await page.locator('#input').focus();await page.keyboard.press(key);await new Promise(r=>setTimeout(r,100));}
async function filename(page,name){await click(page,81932);await key(page,'Control+a');await page.keyboard.insertText(name);await new Promise(r=>setTimeout(r,100));}
async function openDocument(page,name){await action(page,'ui.command',{id:101});await click(page,65548);await key(page,'Control+a');await page.keyboard.insertText(name);await key(page,'Tab');await click(page,'Öffnen');}
async function storage(page){return page.evaluate(()=>new Promise((resolve,reject)=>{const r=indexedDB.open('jfgui-files-v1',1);r.onerror=()=>reject(r.error);r.onsuccess=()=>{const db=r.result,q=db.transaction('files').objectStore('files').getAll();q.onsuccess=()=>{db.close();resolve(q.result.filter(x=>x.app==='PaintCraft').map(x=>({path:x.path,size:x.bytes.byteLength})));};q.onerror=()=>reject(q.error);};}));}
for(const engine of ['chromium','firefox']){
  const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
  const context=await browser.newContext({viewport:{width:1280,height:850},acceptDownloads:true});
  const page=await context.newPage();const messages=[];page.on('pageerror',e=>messages.push(e.message));page.on('console',m=>{if(m.type()==='error')messages.push(m.text());});
  try{
    await page.addInitScript(()=>window.addEventListener('jfgui-ready',()=>window.__jfguiReady=true));
    await page.goto(url+'?agent=1');await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
    await until(()=>state(page),s=>value(s,9901)===1,'initial document');
    await action(page,'ui.command',{id:105}); // Close initial, unmodified image.
    await page.locator('#files').setInputFiles(resolve(root,'apps/PaintCraft/tests/fixtures/a1_rgba.png'));
    await until(()=>state(page),s=>value(s,9913)===64&&value(s,9914)===48,'imported image');
    await key(page,'Control+Shift+n');
    await until(()=>state(page),s=>value(s,9903)===2,'new layer');
    await key(page,'Control+z');await until(()=>state(page),s=>value(s,9903)===1,'undo layer');
    await key(page,'Control+y');await until(()=>state(page),s=>value(s,9903)===2,'redo layer');
    results.push({engine,test:'import-edit-layers-undo-redo',ok:true});
    if(deeper)await page.evaluate(()=>{
      const original=IDBDatabase.prototype.transaction;
      window.__saveConfirmations=[];
      window.__releaseSaves=()=>{IDBDatabase.prototype.transaction=original;for(const fn of window.__saveConfirmations.splice(0))fn();};
      IDBDatabase.prototype.transaction=function(stores,mode,...rest){
        const tx=original.call(this,stores,mode,...rest);
        if(mode==='readwrite')Object.defineProperty(tx,'oncomplete',{set(fn){tx.addEventListener('complete',event=>window.__saveConfirmations.push(()=>fn.call(tx,event)));}});
        return tx;
      };
    });
    await action(page,'ui.command',{id:103});
    await filename(page,`/security-${engine}.pdn`);await click(page,'Speichern');
    await until(()=>state(page),s=>!s.widgets.some(w=>w.id===81932),'save dialog closed');
    if(deeper){
      await until(()=>state(page),s=>isSaving(s),'pending save');
      await page.waitForFunction(()=>window.__saveConfirmations.length===1);
      for(const command of [320,105,102]){try{await action(page,'ui.command',{id:command});}catch(error){if(error.code!=='unavailable')throw error;}}
      const pending=await state(page);
      assert.equal(value(pending,9901),1);assert.equal(value(pending,9903),2);assert.equal(value(pending,9905),1);
      assert.ok(isSaving(pending));
      await page.screenshot({path:resolve(artifact,`${engine}-pending.png`)});
      await page.evaluate(()=>window.__releaseSaves());
      results.push({engine,test:'pending-save-blocks-edit-close-and-second-save',ok:true});
    }
    const saved=await until(()=>state(page),s=>value(s,9905)===0&&!isSaving(s),'storage confirmed'), records=await storage(page);
    results.push({engine,test:'saved-document-is-durable',ok:value(saved,9905)===0&&records.some(x=>x.path===`/security-${engine}.pdn`&&x.size>0),dirty:value(saved,9905),records});
    await writeFile(resolve(artifact,`${engine}-saved-state.json`),JSON.stringify(saved,null,2));
    await page.screenshot({path:resolve(artifact,`${engine}-saved.png`)});
    // A fresh worker must restore the actual saved bytes, including both layers.
    await page.reload();await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
    await until(()=>state(page),s=>value(s,9901)===1,'reloaded');
    await action(page,'ui.command',{id:105});
    await openDocument(page,`/security-${engine}.pdn`);
    await until(()=>state(page),s=>value(s,9903)===2&&value(s,9913)===64&&value(s,9914)===48,'reopened layers');
    results.push({engine,test:'saved-document-survives-reload',ok:true});
    await key(page,'Control+Shift+n');
    await until(()=>state(page),s=>value(s,9903)===3&&value(s,9905)===1,'modified before close');
    await action(page,'ui.command',{id:105});
    await until(()=>state(page),s=>hasLabel(s,'Nicht speichern'),'save before close question');
    await click(page,'Speichern');
    await until(()=>state(page),s=>value(s,9901)===0,'saved then closed');
    await page.reload();await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
    await until(()=>state(page),s=>value(s,9901)===1,'close test reloaded');
    await action(page,'ui.command',{id:105});
    await openDocument(page,`/security-${engine}.pdn`);
    await until(()=>state(page),s=>value(s,9903)===3,'close saved all layers');
    results.push({engine,test:'save-and-close-is-durable',ok:true});
    // Exercise the ordinary error branch without consuming storage or causing a crash.
    await key(page,'Control+Shift+n');
    await until(()=>state(page),s=>value(s,9903)===4&&value(s,9905)===1,'modified again');
    await page.evaluate(()=>{
      const original=IDBDatabase.prototype.transaction;
      IDBDatabase.prototype.transaction=function(stores,mode,...rest){
        if(mode==='readwrite')throw new DOMException('Test: Speichern nicht erlaubt','NotAllowedError');
        return original.call(this,stores,mode,...rest);
      };
    });
    await action(page,'ui.command',{id:102});
    await until(()=>state(page),s=>!isSaving(s)&&s.widgets.some(w=>w.id===9918&&w.label?.length),'save error');
    assert.equal(value(await state(page),9905),1);
    await action(page,'ui.command',{id:105});
    await until(()=>state(page),s=>hasLabel(s,'Nicht speichern'),'unsaved question');
    await click(page,'Speichern');
    await until(()=>state(page),s=>!isSaving(s)&&!hasLabel(s,'Nicht speichern')&&s.widgets.some(w=>w.id===9918&&w.label?.length),'close save error');
    assert.equal(value(await state(page),9901),1);
    assert.equal(value(await state(page),9905),1);
    await action(page,'ui.command',{id:105});
    await until(()=>state(page),s=>hasLabel(s,'Nicht speichern'),'still unsaved question');
    await click(page,'Abbrechen');
    results.push({engine,test:'denied-save-keeps-dirty-document',ok:true});
    // No file-system case assumptions: both small files are supplied through the picker.
    await page.reload();await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
    await until(()=>state(page),s=>value(s,9901)===1,'case test initial');
    await action(page,'ui.command',{id:105});
    const pixels=await readFile(resolve(root,'apps/PaintCraft/tests/fixtures/a1_rgba.png'));
    await page.locator('#files').setInputFiles([{name:'Case.png',mimeType:'image/png',buffer:pixels}]);
    await until(()=>state(page),s=>value(s,9901)===1&&value(s,9913)===64,'upper case image');
    await page.locator('#files').setInputFiles([{name:'case.png',mimeType:'image/png',buffer:pixels}]);
    // With an image open, a normal drop asks whether to open or import it.
    await until(()=>state(page),s=>hasLabel(s,'Öffnen'),'second import question');
    await click(page,'Öffnen');
    await until(()=>state(page),s=>value(s,9901)===2,'case-distinct images');
    results.push({engine,test:'case-distinct-documents',ok:true});
    await action(page,'ui.command',{id:103});
    await filename(page,`/flat-${engine}.png`);await click(page,'Speichern');
    await until(()=>state(page),s=>hasLabel(s,'OK'),'PNG save options');
    await click(page,'OK');
    await until(()=>state(page),s=>value(s,9905)===0&&s.widgets.some(w=>w.id===9911&&w.label===`flat-${engine}.png`),'PNG save confirmed');
    assert.ok((await storage(page)).some(x=>x.path===`/flat-${engine}.png`&&x.size>0));
    results.push({engine,test:'PNG-save-options-is-durable',ok:true});
    if(deeper){
      await key(page,'Control+Shift+n');
      await until(()=>state(page),s=>value(s,9903)===2&&value(s,9905)===1,'first modified PNG');
      await key(page,'Control+Tab');await key(page,'Control+Shift+n');
      await until(()=>state(page),s=>value(s,9903)===2&&value(s,9905)===1,'second modified PNG');
      await action(page,'ui.command',{id:104});
      let completed=false;
      for(let n=0;n<100;n++){
        const s=await state(page);
        if(hasLabel(s,'Zusammenfassen'))await click(page,'Zusammenfassen');
        else if(hasLabel(s,'OK'))await click(page,'OK');
        else if(!isSaving(s)&&value(s,9905)===0){completed=true;break;}
        await new Promise(r=>setTimeout(r,100));
      }
      assert.ok(completed,'save all finished');
      await key(page,'Control+Tab');assert.equal(value(await state(page),9905),0);
      const all=await storage(page);
      assert.ok(all.some(x=>x.path.endsWith('/Case.png')&&x.size>0));
      assert.ok(all.some(x=>x.path===`/flat-${engine}.png`&&x.size>0));
      results.push({engine,test:'save-all-resumes-through-two-PNG-documents',ok:true});
    }
  }catch(error){results.push({engine,test:'scenario',ok:false,error:String(error),messages});await page.screenshot({path:resolve(artifact,`${engine}-failure.png`)});}
  finally{await context.close();await browser.close();}
}
await new Promise(r=>server.close(r));
await writeFile(resolve(artifact,'results.json'),JSON.stringify(results,null,2));
for(const r of results)console.log(JSON.stringify(r));
process.exitCode=results.some(r=>!r.ok)?1:0;
