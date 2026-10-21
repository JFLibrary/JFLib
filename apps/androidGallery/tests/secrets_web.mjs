// The browser's secret store (web/secrets.js, lib/plat/web_secret.rs) in Chromium and Firefox,
// through the gallery's card "05 Sicherer Speicher" (apps/androidGallery/secrets.rs) as a user
// or assistive technology works it (the ARIA shadow tree): store, load, overwrite, a value over
// 4 KB, delete, each surviving a reload; IndexedDB as another script of the origin reads it: a
// non-extractable AES-GCM key, only ciphertext (no value in plain in any database); without a
// secure context the store is Unavailable and the buttons are off. Playwright
// (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/secrets_web.mjs [build] [--engines=chromium,firefox] [--threads] [--headed]
// --threads: cross-origin isolated, the shared-memory WASM variant (?compute=threads)
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright(), threads=args.includes('--threads');
const isolation=threads ? {'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'} : {};
const dir=resolve(build,'web','androidGallery');
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length ? path : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    const bytes=await readFile(file);
    response.writeHead(200,{'content-type':type,'cache-control':'no-store',...isolation}).end(bytes);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}/index.html${threads?'?compute=threads':''}`;

// the records of every IndexedDB database of the page, with bytes as arrays (for the search)
async function dump(page) {
  return page.evaluate(async()=>{
    const names=['jfgui-secrets-v1','jfgui-files-v1'];
    const out={};
    for(const name of names) {
      const db=await new Promise((ok,no)=>{const r=indexedDB.open(name);r.onsuccess=()=>ok(r.result);r.onerror=()=>no(r.error);});
      for(const store of db.objectStoreNames) {
        const all=await new Promise((ok,no)=>{const r=db.transaction(store).objectStore(store).getAll();r.onsuccess=()=>ok(r.result);r.onerror=()=>no(r.error);});
        out[`${name}/${store}`]=all.map(rec=>Object.fromEntries(Object.entries(rec).map(([k,v])=>
          [k,v instanceof CryptoKey ? {crypto:true,extractable:v.extractable,algorithm:v.algorithm.name,length:v.algorithm.length,usages:[...v.usages].sort()}
            : ArrayBuffer.isView(v) || v instanceof ArrayBuffer ? Array.from(new Uint8Array(v.buffer ?? v,v.byteOffset ?? 0,v.byteLength)) : v])));
      }
      db.close();
    }
    return out;
  });
}
const contains=(hay,needle)=>{for(let i=0;i+needle.length<=hay.length;i++){let j=0;while(j<needle.length&&hay[i+j]===needle[j])j++;if(j===needle.length)return true;}return false;};
// every byte array and string of the dump
function blobs(value,out=[]) {
  if(Array.isArray(value) && value.every(x=>typeof x==='number')) out.push(value);
  else if(typeof value==='string') out.push([...new TextEncoder().encode(value)]);
  else if(value && typeof value==='object') for(const v of Object.values(value)) blobs(v,out);
  return out;
}

let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const engines=option('engines','chromium,firefox').split(',');
try {
  for(const engine of engines) {
    const browser=await pw[engine].launch({...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed')});
    try {
      const context=await browser.newContext({viewport:{width:420,height:900}});
      const page=await context.newPage();
      const errors=[];
      page.on('pageerror',e=>errors.push(String(e)));
      page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
      const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
      const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
      const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`${engine}: timed out: ${what}`);};
      const field=()=>page.locator('#a11y input[type=text]').nth(2); // 03 name, 04 note, 05 secret
      const open=async()=>{
        await page.goto(base);
        await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Sichern"]'));
        if(threads) assert.equal(await page.evaluate(()=>document.body.dataset.compute),'threads','the threads variant runs');
      };
      const click=async(name,expect,what)=>{await el(name).dispatchEvent('click');return until(async()=>{const t=await text(expect);return t;},what);};
      const enter=async value=>{
        const f=field();
        await f.focus();
        const key=await f.getAttribute('data-key');
        await until(async()=>(await page.evaluate(()=>window.jfguiA11y.mirror.active()?.focus))===key,'focus on the secret field');
        // the keyboard through the host's #input (the app keeps its focus on the field)
        await page.locator('#input').focus();
        ok((await page.evaluate(()=>window.jfguiA11y.mirror.active()?.focus))===key,'the app keeps the focus');
        await page.keyboard.press('Control+A');await page.keyboard.press('Backspace');
        await until(async()=>await f.inputValue()==='','field emptied');
        // in pieces: the UI takes at most TEXTQ_MAX (64) characters of text per pass
        for(let at=0;at<value.length;at+=60) {
          // the DOM focus follows the app into the shadow tree after keys and updates; text
          // inserted into the shadow field did not reach the app in Firefox: through #input
          await page.locator('#input').focus();
          await page.keyboard.insertText(value.slice(at,at+60));
          const want=value.slice(0,at+60);
          await until(async()=>await f.inputValue()===want,`typed ${want.length} of ${value.length} characters`).catch(async e=>{const v=await f.inputValue();const act=await page.evaluate(()=>{const a=document.activeElement;return a.id||a.tagName+':'+a.getAttribute('aria-label');});throw new Error(e.message+` (got ${v.length}: ${JSON.stringify(v.slice(0,100))}; DOM focus ${act})`);});
        }
      };
      // the stored record (app, service, account) once the host committed the change
      const record=()=>dump(page).then(d=>d['jfgui-secrets-v1/values'].find(r=>r.app==='androidGallery' && r.service==='galerie' && r.account==='demo'));

      await open();
      ok(await text('Der Wert liegt verschlüsselt')!==null,`${engine}: the store is available (secret_available)`);
      ok(await el('Sichern').getAttribute('aria-disabled')!=='true','buttons on');
      await click('Laden','Kein Geheimnis gesichert.','nothing stored yet');
      checks++;

      // store: the field empties, IndexedDB gets the encrypted record
      const first='hunter2 – Passwort ✓';
      const firstBytes=[...new TextEncoder().encode(first)];
      await enter(first);
      await click('Sichern',`Gesichert (${firstBytes.length} Bytes).`,'stored');
      ok(await field().inputValue()==='','the field is emptied after storing');
      let rec=await until(record,'record committed to IndexedDB');
      ok(rec.iv.length===12 && rec.data.length===firstBytes.length+16,`${engine}: IV of 12 bytes, ciphertext + 16-byte tag (${rec.data.length})`);
      let d=await dump(page);
      const keys=d['jfgui-secrets-v1/keys'];
      ok(keys.length===1 && keys[0].app==='androidGallery','one key, of the app');
      const k=keys[0].key;
      ok(k.crypto && k.extractable===false && k.algorithm==='AES-GCM' && k.length===256 && k.usages.join()==='decrypt,encrypt',`${engine}: a non-extractable AES-GCM 256 CryptoKey: ${JSON.stringify(k)}`);
      ok(await page.evaluate(async()=>{
        const db=await new Promise(ok=>{const r=indexedDB.open('jfgui-secrets-v1');r.onsuccess=()=>ok(r.result);});
        const key=await new Promise(ok=>{const r=db.transaction('keys').objectStore('keys').get('androidGallery');r.onsuccess=()=>ok(r.result.key);});
        db.close();
        try {await crypto.subtle.exportKey('raw',key);return false;} catch(e) {return e.name==='InvalidAccessError';}
      }),'exportKey refuses the key (InvalidAccessError)');
      ok(!blobs(d).some(b=>contains(b,firstBytes)),`${engine}: the value is nowhere in plain in IndexedDB`);
      await click('Laden',`Geladen (${firstBytes.length} Bytes).`,'loaded');
      ok(await field().inputValue()===first,'get returns the value');

      // overwrite: a new IV, the new value
      const second='correct horse battery staple';
      await enter(second);
      await click('Sichern',`Gesichert (${second.length} Bytes).`,'overwritten');
      rec=await until(async()=>{const r=await record();return r && r.data.length===second.length+16 && r;},'overwrite committed');
      ok(rec.data.length===second.length+16,'the record holds the new value');
      await click('Laden',`Geladen (${second.length} Bytes).`,'loaded again');
      ok(await field().inputValue()===second,'get returns the new value');

      // a reload: the host decrypts the stored value for the new instance
      await open();
      await click('Laden',`Geladen (${second.length} Bytes).`,'loaded after a reload');
      ok(await field().inputValue()===second,`${engine}: the value survives a reload`);

      // over 4 KB
      const large=Array.from({length:5200},(_,i)=>String.fromCharCode(33+(i*7919)%94)).join('');
      await enter(large);
      await click('Sichern',`Gesichert (${large.length} Bytes).`,'large stored');
      await until(async()=>{const r=await record();return r && r.data.length===large.length+16;},'large committed',20000);
      checks++;
      d=await dump(page);
      ok(!blobs(d).some(b=>contains(b,[...new TextEncoder().encode(large.slice(0,32))])),'the large value is not in plain in IndexedDB');
      await open();
      await click('Laden',`Geladen (${large.length} Bytes).`,'large loaded after a reload');
      ok(await field().inputValue()===large,`${engine}: ${large.length} bytes survive a reload`);

      // a damaged record (another script of the origin changes the ciphertext): refused, not plain
      await page.evaluate(async()=>{
        const db=await new Promise(ok=>{const r=indexedDB.open('jfgui-secrets-v1');r.onsuccess=()=>ok(r.result);});
        const t=db.transaction('values','readwrite'),s=t.objectStore('values');
        const r=s.get(['androidGallery','galerie','demo']);
        r.onsuccess=()=>{const v=r.result;v.data=new Uint8Array(v.data);v.data[0]^=1;s.put(v);};
        await new Promise(ok=>t.oncomplete=ok);db.close();
      });
      await open();
      await click('Laden','Nicht geladen:','a damaged record');
      ok((await text('Nicht geladen:')).includes('does not decrypt'),`${engine}: a changed record is refused (Failed): ${await text('Nicht geladen:')}`);

      // delete: gone, also after a reload
      await click('Löschen','Gelöscht.','deleted');
      await until(async()=>!(await record()),'delete committed');
      checks++;
      await click('Laden','Kein Geheimnis gesichert.','nothing after delete');
      await open();
      await click('Laden','Kein Geheimnis gesichert.','nothing after delete and reload');
      await click('Löschen','Es war keines gesichert.','delete of a missing entry');
      checks+=3;
      ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);

      // no secure context (a page over plain http from another host): no store, nothing written
      const insecure=await browser.newContext({viewport:{width:420,height:900}});
      const p2=await insecure.newPage();
      await p2.addInitScript(()=>Object.defineProperty(window,'isSecureContext',{value:false}));
      await p2.goto(base);
      await p2.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Sichern"]'));
      ok(await p2.evaluate(()=>[...document.querySelectorAll('#a11y [data-key]')].some(e=>e.textContent==='Kein sicherer Speicher verfügbar.')),`${engine}: without a secure context: not available`);
      ok(await p2.locator('#a11y [aria-label="Sichern"]').first().getAttribute('aria-disabled')==='true','... and the buttons are off');
      ok(!(await p2.evaluate(async()=>(await indexedDB.databases?.() ?? []).some(d=>d.name==='jfgui-secrets-v1'))),'... and no secret database is created');
      await insecure.close();
      await context.close();
      console.log(`secrets web (${engine}${threads?', threads':''}): passed`);
    } finally {await browser.close();}
  }
  console.log(`secrets web: ${checks} checks passed (${engines.join(', ')}: store, load, overwrite, reload, 5200 bytes, damaged record, delete, IndexedDB holds only ciphertext and a non-extractable key, insecure context)`);
} finally {
  server.close();
}
