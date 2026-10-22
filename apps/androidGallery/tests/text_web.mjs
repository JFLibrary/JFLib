// Text input in the browser (web/worker.js, web/loader.js, web/a11y.js, lib/plat/web.rs
// jfgui_text, lib/ui/ctx.rs's text queue) in Chromium and Firefox, through the gallery's field
// "05 Sicherer Speicher" (8 KB, apps/androidGallery/secrets.rs): a long insert (5000
// characters with emoji, a ZWJ sequence, a combining sequence, CJK and numbered marks) arrives
// complete and in order, through the host's #input and through the field of the ARIA shadow
// tree; typing, Backspace, Delete, a paste and an IME composition in the shadow tree's field
// reach the app. Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from
// PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/text_web.mjs [build] [--engines=chromium,firefox] [--threads] [--headed]
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

// 5000 code points: numbered blocks (a lost or moved piece shows), emoji (surrogate pairs), a
// ZWJ sequence, e + combining acute, CJK, umlauts
function long(count) {
  const parts=[];
  for(let i=0,n=0;n<count;i++) {
    const block=`#${String(i).padStart(4,'0')} Grüße aus 東京 🙂 é 👩‍💻 abcdefghij `;
    const cps=Array.from(block);
    parts.push(cps.slice(0,count-n).join(''));
    n+=Math.min(cps.length,count-n);
  }
  return parts.join('');
}
const LONG=long(5000);
assert.equal(Array.from(LONG).length,5000);
assert.ok(Buffer.byteLength(LONG)<=8192,'fits the field (8192 bytes)');

let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const engines=option('engines','chromium,firefox').split(',');
try {
  for(const engine of engines) {
    const browser=await pw[engine].launch({...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed')});
    try {
      const page=await browser.newPage({viewport:{width:420,height:900}});
      const errors=[];
      page.on('pageerror',e=>errors.push(String(e)));
      page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
      const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`${engine}: timed out: ${what}`);};
      await page.goto(base);
      await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Sichern"]'));
      if(threads) assert.equal(await page.evaluate(()=>document.body.dataset.compute),'threads','the threads variant runs');
      const field=page.locator('#a11y input[type=text]').nth(2); // 03 name, 04 note, 05 secret
      const key=await field.getAttribute('data-key');
      // the app's text of the field (the mirror of its accessibility tree)
      const app=()=>page.evaluate(k=>window.jfguiA11y.mirror.wins.get(0).nodes.get(k)?.text,key);
      const active=()=>page.evaluate(()=>{const a=document.activeElement;return a?.id==='input'?'#input':a?.dataset.key ?? a?.tagName;});
      const show=s=>s===undefined ? 'nothing' : `${Array.from(s).length} characters ${JSON.stringify(s.length>80 ? s.slice(0,40)+' … '+s.slice(-40) : s)}`;
      const expect=async(want,what,ms)=>{
        await until(async()=>await app()===want && await field.inputValue()===want,what,ms).catch(async e=>{
          const got=await app(), dom=await field.inputValue();
          let at=0;while(at<want.length && got?.[at]===want[at])at++;
          throw new Error(`${e.message}: the app has ${show(got)}, the field ${show(dom)}, first difference at ${at}; DOM focus ${await active()}`);
        });
        checks++;
      };
      // the DOM focus in the shadow tree's field, the app's focus follows
      const focusField=async()=>{
        await field.focus();
        await until(async()=>(await page.evaluate(()=>window.jfguiA11y.mirror.active()?.focus))===key,'app focus on the field');
      };
      const clear=async()=>{await page.keyboard.press('Control+A');await page.keyboard.press('Backspace');await expect('','field emptied');};

      // typing, Backspace, Delete with the DOM focus in the shadow tree's field
      await focusField();
      await clear();
      await page.keyboard.type('Grüße 🙂x');
      await expect('Grüße 🙂x','typed into the shadow field');
      await page.keyboard.press('Backspace');
      await expect('Grüße 🙂','Backspace in the shadow field');
      await page.keyboard.press('Home');
      await page.keyboard.press('Delete');
      await expect('rüße 🙂','Delete in the shadow field');
      ok(await active()===key,`${engine}: the DOM focus stays in the shadow field`);
      // inserted text (Firefox: a composition whose input event comes after compositionend)
      await page.keyboard.press('End');
      await page.keyboard.insertText(' 日本');
      await expect('rüße 🙂 日本','inserted into the shadow field');
      // a paste (a synthetic event: the system clipboard stays untouched)
      const payload=await field.evaluate(f=>{
        const data=new DataTransfer();data.setData('text/plain',' Paste ✓');
        const event=new ClipboardEvent('paste',{bubbles:true,cancelable:true,clipboardData:data});
        // Firefox hides the data of an untrusted paste: the payload only, the handlers stay real
        if(!event.clipboardData?.getData('text/plain')) Object.defineProperty(event,'clipboardData',{value:{getData:t=>t==='text/plain'?' Paste ✓':''}});
        f.dispatchEvent(event);
        return event.defaultPrevented;
      });
      ok(payload,`${engine}: the host takes the paste`);
      await expect('rüße 🙂 日本 Paste ✓','pasted into the shadow field');
      // an IME composition: the field shows it, the app gets the committed text once
      if(engine==='chromium') {
        const cdp=await page.context().newCDPSession(page);
        await cdp.send('Input.imeSetComposition',{text:'にほん',selectionStart:3,selectionEnd:3});
        await until(async()=>await field.inputValue()==='rüße 🙂 日本 Paste ✓にほん','the composition in the field');
        ok(await app()==='rüße 🙂 日本 Paste ✓','the app does not get the composition before it ends');
        await cdp.send('Input.insertText',{text:'日本'});
        await expect('rüße 🙂 日本 Paste ✓日本','the committed composition');
      } else {
        // Firefox: keyboard.insertText is a composition (start, update, end, then input)
        await page.keyboard.insertText('日本');
        await expect('rüße 🙂 日本 Paste ✓日本','the committed composition');
      }

      // a long insert into the shadow tree's field
      await clear();
      await page.keyboard.insertText(LONG);
      await expect(LONG,'5000 characters inserted into the shadow field',30000);
      ok(await active()===key,`${engine}: the DOM focus stays in the shadow field`);
      // ... and through the host's #input (the keyboard after a pointer on the canvas)
      await clear();
      await page.locator('#input').focus();
      ok((await page.evaluate(()=>window.jfguiA11y.mirror.active()?.focus))===key,'the app keeps the focus');
      await page.keyboard.insertText(LONG);
      await expect(LONG,'5000 characters inserted through #input',30000);
      // typed after it, in order
      await page.keyboard.type('Ende');
      await expect(LONG+'Ende','typed after the long insert');
      ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
      console.log(`text web (${engine}${threads?', threads':''}): passed`);
    } finally {await browser.close();}
  }
  console.log(`text web: ${checks} checks passed (${engines.join(', ')}: typing, Backspace, Delete, insert, paste and IME composition in the shadow tree's field, 5000 characters through it and through #input)`);
} finally {
  server.close();
}
