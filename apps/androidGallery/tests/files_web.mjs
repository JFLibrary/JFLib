// The user's own files in the browser (web/handles.js, web/loader.js, lib/plat/web.rs,
// web_files.rs) through the gallery's card "04 Dateien & Zwischenablage" (apps/androidGallery/
// files.rs), as a user works it (the ARIA shadow tree). Chromium: the system's pickers stubbed
// to hand out files of the origin private file system (a real picker cannot be automated) --
// open a file (imported, bound to /handles/ID/NAME, the handle in IndexedDB), save as a new file
// and over the opened one (written back at once; the same path for the same file), a failed
// write leaves the file as it was, a reload reads the remembered file again (permission still
// granted: at the start; "prompt": ui.recent_open asks from the click, UIC_RECENT granted /
// denied), the recent list survives every reload. Firefox: no File System Access API -- the
// library's own dialog over the virtual files, no handle database; the recent list survives a
// reload (the file itself was not kept: "Nicht mehr da", it leaves the list), and the emptied
// list stays empty after the next reload.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/files_web.mjs [build] [--engines=chromium,firefox] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright();
const dir=resolve(build,'web','androidGallery');
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length ? path : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store'}).end(await readFile(file));
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}/index.html`;

// the pickers hand out files of the origin private file system; sessionStorage steers them and
// the permissions ("perm": granted / denied after a "prompt"; "fail": every write fails)
function stubs() {
  const calls=[];
  Object.defineProperty(window,'__pickerCalls',{value:calls});
  const opfs=()=>navigator.storage.getDirectory();
  const plain=o=>({types:o?.types,suggestedName:o?.suggestedName,startIn:o?.startIn?.name,id:o?.id,multiple:o?.multiple});
  window.showOpenFilePicker=async o=>{
    calls.push({kind:'open',...plain(o)});
    const n=sessionStorage.getItem('open') || 'brief.txt';
    if(n==='cancel') throw new DOMException('abgebrochen','AbortError');
    return [await (await opfs()).getFileHandle(n)];
  };
  window.showSaveFilePicker=async o=>{
    calls.push({kind:'save',...plain(o)});
    return (await opfs()).getFileHandle(sessionStorage.getItem('save') || o?.suggestedName || 'gespeichert.txt',{create:true});
  };
  const query=FileSystemHandle.prototype.queryPermission, ask=FileSystemHandle.prototype.requestPermission;
  FileSystemHandle.prototype.queryPermission=function(d) {return sessionStorage.getItem('perm') ? Promise.resolve('prompt') : query.call(this,d);};
  FileSystemHandle.prototype.requestPermission=function(d) {
    calls.push({kind:'ask',mode:d?.mode});
    const p=sessionStorage.getItem('perm');
    return p ? Promise.resolve(p) : ask.call(this,d);
  };
  const writable=FileSystemFileHandle.prototype.createWritable;
  FileSystemFileHandle.prototype.createWritable=async function(o) {
    const w=await writable.call(this,o);
    if(sessionStorage.getItem('fail')) {
      const write=w.write.bind(w);
      w.write=async()=>{await write(new Uint8Array(3));throw new DOMException('Schreibfehler','InvalidStateError');};
    }
    return w;
  };
}

let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const engines=option('engines','chromium,firefox').split(',');
try {
  for(const engine of engines) {
    const browser=await pw[engine].launch({...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed')});
    try {
      const context=await browser.newContext({viewport:{width:420,height:1400}});
      const chromium=engine==='chromium';
      if(chromium) await context.addInitScript(stubs);
      const page=await context.newPage();
      const errors=[];
      page.on('pageerror',e=>errors.push(String(e)));
      const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
      const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
      const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`${engine}: timed out: ${what}`);};
      const open=async()=>{
        await page.goto(base);
        await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Speichern…"]'));
      };
      const click=name=>el(name).dispatchEvent('click');
      const note=()=>page.locator('#a11y input[type=text]').nth(1); // 03 name, 04 note
      const enter=async value=>{
        const f=note();
        await f.focus();
        const key=await f.getAttribute('data-key');
        await until(async()=>(await page.evaluate(()=>window.jfguiA11y.mirror.active()?.focus))===key,'focus on the note');
        await page.keyboard.press('Control+A');await page.keyboard.press('Backspace');
        await until(async()=>await f.inputValue()==='','note emptied');
        await page.keyboard.insertText(value);
        await until(async()=>await f.inputValue()===value,'note typed');
      };
      const opfs={
        write:(name,value)=>page.evaluate(async([n,v])=>{const h=await (await navigator.storage.getDirectory()).getFileHandle(n,{create:true});const w=await h.createWritable();await w.write(v);await w.close();},[name,value]),
        read:name=>page.evaluate(async n=>{try{return await (await (await (await navigator.storage.getDirectory()).getFileHandle(n)).getFile()).text();}catch{return null;}},name),
      };
      const records=()=>page.evaluate(async()=>{
        if(!(await indexedDB.databases()).some(d=>d.name==='jfgui-handles-v1')) return null;
        const db=await new Promise((ok,no)=>{const r=indexedDB.open('jfgui-handles-v1');r.onsuccess=()=>ok(r.result);r.onerror=()=>no(r.error);});
        const all=await new Promise((ok,no)=>{const r=db.transaction('handles').objectStore('handles').getAll();r.onsuccess=()=>ok(r.result);r.onerror=()=>no(r.error);});
        db.close();
        return all.map(r=>({app:r.app,path:r.path,name:r.handle?.name,kind:r.handle?.kind}));
      });
      const calls=()=>page.evaluate(()=>window.__pickerCalls.slice());

      await open();
      if(chromium) {
        const letter='Hallo aus der Datei ✓';
        await opfs.write('brief.txt',letter);
        // open: the picker's file imported and bound
        await click('Öffnen…');
        const opened=await until(()=>text('Geöffnet: '),'opened');
        ok(opened==="Geöffnet: brief.txt",`${engine}: the chosen file opened (${opened})`);
        ok(await note().inputValue()===letter,'its text in the note');
        let c=await calls();
        ok(c[0]?.kind==='open' && JSON.stringify(c[0].types)===JSON.stringify([{description:'.txt',accept:{'*/*':['.txt']}}]) && c[0].multiple===false && /^jfgui-androidGallery$/.test(c[0].id),
          `the open picker with the dialog's types: ${JSON.stringify(c[0])}`);
        let r=await records();
        ok(r?.length===1 && /^\/handles\/[0-9a-f]{8}\/brief\.txt$/.test(r[0].path) && r[0].app==='androidGallery' && r[0].name==='brief.txt' && r[0].kind==='file',
          `the handle kept in IndexedDB with its path: ${JSON.stringify(r)}`);
        const briefPath=r[0].path;
        ok(await text('Zuletzt: brief.txt')!==null,'the file is first in the recent list');

        // save as a new file: written back at once
        await enter('Neuer Text für die Notiz');
        await page.evaluate(()=>sessionStorage.setItem('save','Notiz.txt'));
        await click('Speichern…');
        await until(()=>text('Gespeichert: Notiz.txt'),'saved as');
        await until(async()=>await opfs.read('Notiz.txt')==='Neuer Text für die Notiz','written into the new file');
        checks++;
        c=await calls();
        const save=c.find(x=>x.kind==='save');
        ok(save?.suggestedName==='Notiz.txt' && save.types?.[0]?.accept?.['*/*']?.[0]==='.txt',`the save picker: the suggested name and types: ${JSON.stringify(save)}`);
        r=await records();
        ok(r.length===2 && r.some(x=>x.name==='Notiz.txt' && x.path.endsWith('/Notiz.txt')),'a second handle, its own path');

        // save over the opened file: the same handle, the same path, the file replaced
        await enter('Überschrieben ✓');
        await page.evaluate(()=>sessionStorage.setItem('save','brief.txt'));
        await click('Speichern…');
        await until(async()=>await opfs.read('brief.txt')==='Überschrieben ✓','written back into the opened file');
        checks++;
        r=await records();
        ok(r.length===2 && r.find(x=>x.name==='brief.txt').path===briefPath,`the same file keeps its path (${JSON.stringify(r)})`);

        // a write that fails half way: the file stays as it was (the browser's swap file)
        await page.evaluate(()=>sessionStorage.setItem('fail','1'));
        await enter('Darf nicht ankommen');
        await click('Speichern…');
        await until(async()=>(await page.locator('#status').textContent()).includes('Schreibfehler'),'the failed write reported');
        ok(await opfs.read('brief.txt')==='Überschrieben ✓','a failed write leaves the file as it was');
        await page.evaluate(()=>sessionStorage.removeItem('fail'));

        // a reload: the remembered file is read at the start (the permission is still granted)
        await opfs.write('brief.txt','Von außen geändert');
        await open();
        ok(await text('Zuletzt: brief.txt')!==null,`${engine}: the recent list survives the reload`);
        await click('Zuletzt öffnen');
        await until(()=>text('Wieder geöffnet: brief.txt'),'opened again after the reload');
        ok(await note().inputValue()==='Von außen geändert','the file read again at the start: its current contents');
        ok(!(await calls()).some(x=>x.kind==='open' || x.kind==='save'),'no picker after the reload');

        // the permission back at "prompt": nothing read at the start; reopen asks (from the click)
        await page.evaluate(()=>sessionStorage.setItem('perm','granted'));
        await opfs.write('brief.txt','Nach der Rückfrage');
        await open();
        await click('Zuletzt öffnen');
        await until(()=>text('Wieder geöffnet: brief.txt'),'reopened after the question');
        ok(await note().inputValue()==='Nach der Rückfrage','reopen read the file after the permission was granted');
        ok((await calls()).some(x=>x.kind==='ask' && x.mode==='read'),'reopen asked for reading');
        // and written back after asking for writing
        await enter('Mit Rückfrage gespeichert');
        await page.evaluate(()=>sessionStorage.setItem('save','brief.txt'));
        await click('Speichern…');
        await until(async()=>await opfs.read('brief.txt')==='Mit Rückfrage gespeichert','written back after the question');
        ok((await calls()).some(x=>x.kind==='ask' && x.mode==='readwrite'),'the write asked for writing');
        // denied: reopen fails, nothing imported
        await page.evaluate(()=>sessionStorage.setItem('perm','denied'));
        await open();
        await click('Zuletzt öffnen');
        const denied=await until(()=>text('Nicht wieder geöffnet'),'reopen denied');
        ok(denied.includes('abgelehnt') && denied.includes('brief.txt'),`a refused permission: ${denied}`);
        ok(await text('Zuletzt: ')!==null,'a refused permission keeps the list');
        await page.evaluate(()=>sessionStorage.removeItem('perm'));

        // a cancelled picker: nothing chosen
        await page.evaluate(()=>sessionStorage.setItem('open','cancel'));
        await click('Öffnen…');
        await until(async()=>(await calls()).some(x=>x.kind==='open'),'the picker shown');
        await page.waitForTimeout(300);
        ok(await text('Geöffnet: ')===null,'a cancelled picker opens nothing');
      } else {
        // no File System Access API: the library's own dialog over the virtual files
        ok(await page.evaluate(()=>typeof window.showOpenFilePicker)==='undefined',`${engine}: no pickers`);
        await click('Speichern…');
        await until(async()=>await el('Speichern').count()>0 && await text('Speichern unter')!==null,'the own save dialog');
        checks++;
        await click('Speichern');
        const saved=await until(()=>text('Gespeichert: Notiz.txt'),'saved in the own dialog');
        ok(saved==="Gespeichert: Notiz.txt",`${engine}: saved into the virtual files (${saved})`);
        ok(await text('Zuletzt: Notiz.txt')!==null,`${engine}: the file is first in the recent list`);
        // a reload: the list is kept in the browser's storage, the file itself was not
        await open();
        ok(await text('Zuletzt: Notiz.txt')!==null,`${engine}: the recent list survives the reload`);
        await click('Zuletzt öffnen');
        const gone=await until(()=>text('Nicht mehr da: ')||text('Wieder geöffnet: '),'the recent file tried');
        ok(gone==='Nicht mehr da: Notiz.txt',`${engine}: a file the app did not keep is gone, said so (${gone})`);
        await until(async()=>await text('Zuletzt: ')===null,'out of the list');
        checks++;
        await open();
        ok(await text('Zuletzt: ')===null,`${engine}: the emptied list stays empty after a reload`);
        await click('Öffnen…');
        await until(async()=>await text('Öffnen')!==null && await el('Abbrechen').count()>0,'the own open dialog');
        await click('Abbrechen');
        ok(await records()===null,`${engine}: no handle database`);
      }
      ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
      console.log(`${engine}: passed`);
    } finally {await browser.close();}
  }
  console.log(`files_web: ${checks} checks passed`);
} finally {server.close();}
