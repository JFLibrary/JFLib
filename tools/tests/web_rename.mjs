// Namespace rename acceptance: real IndexedDB migration, conflict policy and durable retry.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve,extname,sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from './playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-table-formulas'),folder=resolve(build,'web/jfWord'),output=resolve(build,'rename-browser');
await mkdir(output,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const source=await readFile(resolve(build,'formulas-browser/chromium-light-1.docx'));
const server=createServer(async(req,res)=>{
  try {
    const name=decodeURIComponent(new URL(req.url,'http://localhost').pathname).slice(1);
    if(name==='seed'){res.writeHead(200,{'content-type':'text/html'}).end('<!doctype html><title>Storage fixture</title>');return;}
    if(name==='favicon.ico'){res.writeHead(204).end();return;}
    const path=resolve(folder,name || 'index.html');if(!path.startsWith(folder+sep))throw Error('Invalid path');
    res.writeHead(200,{'content-type':{'.js':'text/javascript','.html':'text/html','.css':'text/css','.wasm':'application/wasm'}[extname(path)] || 'application/octet-stream','cache-control':'no-store'}).end(await readFile(path));
  }catch(error){res.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));const base=`http://127.0.0.1:${server.address().port}`;
let serial=0;const results=[];
async function raw(page,method,params={}){return page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++serial,method,params});}
async function rpc(page,method,params={}){const reply=await raw(page,method,params);assert.ok(!reply.error,JSON.stringify(reply));return reply.result;}
const state=page=>rpc(page,'state/inspect',{domain:'document',limit:100});
const action=async(page,action,input={})=>rpc(page,'actions/execute',{action,input,expectedRevision:(await state(page)).revision});
async function exported(page){const uri=(await action(page,'document.export')).resources[0].uri;let offset=0;const chunks=[];for(;;){const data=await rpc(page,'resources/read',{uri,offset,limit:65536});chunks.push(Buffer.from(data.data,'base64'));if(data.eof)return Buffer.concat(chunks);offset=data.nextOffset;}}
async function database(page,name){return page.evaluate(name=>new Promise((resolve,reject)=>{
  const r=indexedDB.open(name,1);r.onerror=()=>reject(r.error);r.onsuccess=()=>{const db=r.result,t=db.transaction('files','readonly'),get=t.objectStore('files').getAll();t.oncomplete=()=>{db.close();resolve(get.result.map(f=>({key:f.key,app:f.app,path:f.path,bytes:[...new Uint8Array(f.bytes)]})));};t.onerror=()=>reject(t.error);};
}),name);}
async function ready(page){await page.goto(`${base}/?agent=1&compute=scalar`);await page.waitForFunction(()=>typeof window.jfguiAgent==='object',null,{timeout:45000});}
try {
  for(const engine of ['chromium','firefox']){
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try{for(const mode of ['normal','quota']){
      const context=await browser.newContext();
      try{
        const page=await context.newPage();await page.goto(base+'/seed');
        await page.evaluate(async({bytes,mode})=>{
          if(mode==='quota')sessionStorage.setItem('migration-quota','1');
          function put(name,records){return new Promise((resolve,reject)=>{const r=indexedDB.open(name,1);r.onupgradeneeded=()=>r.result.createObjectStore('files',{keyPath:'key'});r.onerror=()=>reject(r.error);r.onsuccess=()=>{const db=r.result,t=db.transaction('files','readwrite');for(const f of records)t.objectStore('files').put(f);t.oncomplete=()=>{db.close();resolve();};t.onerror=()=>reject(t.error);};});}
          const file={app:'jfWord',path:'/documents/legacy.docx',key:'jfWord:/documents/legacy.docx',bytes:new Uint8Array(bytes).buffer};
          await put('asmgui-files-v1',[file,{...file,key:'jfWord:/documents/current.docx',path:'/documents/current.docx',bytes:new Uint8Array([1,2,3]).buffer},{key:'gallery:/shared.bin',app:'gallery',path:'/shared.bin',bytes:new Uint8Array([4,5,6]).buffer}]);
          await put('jfgui-files-v1',[{...file,key:'jfWord:/documents/current.docx',path:'/documents/current.docx'}]);
        },{bytes:[...source],mode});
        await page.addInitScript(()=>{
          const transaction=IDBDatabase.prototype.transaction;
          IDBDatabase.prototype.transaction=function(...args){if(this.name==='jfgui-files-v1'&&args[1]==='readwrite'&&sessionStorage.getItem('migration-quota'))throw new DOMException('Migration quota test','QuotaExceededError');return transaction.apply(this,args);};
        });
        await ready(page);assert.equal(await page.evaluate(()=>typeof window.asmguiAgent),'undefined');
        await action(page,'file.open',{path:'/documents/legacy.docx'});assert.deepEqual(await exported(page),source);
        await action(page,'file.open',{path:'/documents/current.docx'});assert.deepEqual(await exported(page),source,'new namespace file must win over conflicting legacy data');
        const original=await database(page,'asmgui-files-v1');assert.equal(original.length,3,'legacy source retained');
        if(mode==='quota'){
          assert.equal((await database(page,'jfgui-files-v1')).length,1,'failed migration cannot partially commit');
          const reply=await raw(page,'actions/execute',{action:'file.save',input:{path:'/documents/retry.docx'},expectedRevision:(await state(page)).revision});assert.ok(reply.error,'durable save unavailable before migration commit');
          await page.evaluate(()=>sessionStorage.removeItem('migration-quota'));await ready(page);
        }
        const migrated=await database(page,'jfgui-files-v1');assert.equal(migrated.length,3);
        assert.deepEqual(Buffer.from(migrated.find(f=>f.path==='/documents/current.docx').bytes),source);
        assert.deepEqual(migrated.find(f=>f.app==='gallery').bytes,[4,5,6],'migration preserves other app records');
        await action(page,'file.open',{path:'/documents/legacy.docx'});assert.deepEqual(await exported(page),source);
        await ready(page);assert.equal((await database(page,'jfgui-files-v1')).length,3,'migration idempotent');
        assert.deepEqual(await database(page,'asmgui-files-v1'),original);
        results.push({engine,mode,legacyRead:true,sourceRetained:true,newerWins:true,otherApps:true,atomicFailure:mode==='quota',durableRetry:mode==='quota',idempotent:true});
        console.log(`PASS ${engine} ${mode}: jfgui WASM ABI, old files, conflicts, cross-app storage and durable migration`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
