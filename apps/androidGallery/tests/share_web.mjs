// The share sheet in the browser (lib/plat/share.rs, web_share.rs, web/loader.js) through the gallery's
// card "06 System" (apps/androidGallery/system.rs), as a user works it (the ARIA shadow tree).
// Chromium: navigator.share stubbed (the real one opens the system's sheet, which cannot be automated):
// the button "Teilen" hands over the title, the text, the link and the file (a File with the file's
// bytes) -> "geteilt" when the share resolves; "abgebrochen" when it rejects with AbortError,
// "fehlgeschlagen" for another error; without navigator.share the page grants no CAP_SHARE and the
// button answers "nicht möglich" at once.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/share_web.mjs [build] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
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

// navigator.share stubbed: every call recorded (its files read as text); sessionStorage "share" says
// what it does: "ok" resolves, "cancel" rejects with AbortError, "fail" with a TypeError, "none":
// no navigator.share at all
function stubs() {
  if(sessionStorage.getItem('share')==='none') {try{delete Navigator.prototype.share;delete Navigator.prototype.canShare;}catch{} return;}
  const calls=[];
  Object.defineProperty(window,'__shareCalls',{value:calls});
  Navigator.prototype.canShare=function(d) {return !d?.files || d.files.every(f=>f instanceof File);};
  Navigator.prototype.share=async function(d) {
    const files=[];
    for(const f of d?.files || []) files.push({name:f.name,size:f.size,text:await f.text()});
    calls.push({title:d?.title,text:d?.text,url:d?.url,files});
    const how=sessionStorage.getItem('share') || 'ok';
    if(how==='cancel') throw new DOMException('abgebrochen','AbortError');
    if(how==='fail') throw new TypeError('kaputt');
  };
}

let failed=0;
const browser=await launchWasm(pw.chromium,{...chromiumOptions(),headless:!args.includes('--headed')});
try {
  const context=await browser.newContext();
  await context.addInitScript(stubs);
  const page=await context.newPage();
  const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
  const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`timed out: ${what}`);};
  const shown=async text=>(await page.locator('#a11y').textContent()).includes(text);
  const step=async(what,fn)=>{try{await fn();console.log(`  ok   ${what}`);}catch(e){failed++;console.log(`  FAIL ${what}: ${e.message}`);}};
  const load=async how=>{
    await page.goto(base);
    await page.evaluate(h=>sessionStorage.setItem('share',h),how);
    await page.reload();
    await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Teilen"]'));
  };

  await step('shared: the title, the text, the link and the file reach navigator.share -> "geteilt"',async()=>{
    await load('ok');
    await el('Teilen').dispatchEvent('click');
    await until(()=>shown('Teilen: geteilt'),'"Teilen: geteilt"');
    const calls=await page.evaluate(()=>window.__shareCalls);
    assert.equal(calls.length,1);
    assert.equal(calls[0].title,'JFLib Galerie');
    assert.equal(calls[0].text,'Geteilt aus der JFLib Galerie');
    assert.equal(calls[0].url,'https://jflib.org/');
    assert.deepEqual(calls[0].files,[{name:'Galerie-Teilen.txt',size:36,text:'JFLib Galerie: eine Datei zum Teilen'}]);
  });
  await step('closed without a target (AbortError) -> "abgebrochen"',async()=>{
    await load('cancel');
    await el('Teilen').dispatchEvent('click');
    await until(()=>shown('Teilen: abgebrochen'),'"Teilen: abgebrochen"');
  });
  await step('another error -> "fehlgeschlagen"',async()=>{
    await load('fail');
    await el('Teilen').dispatchEvent('click');
    await until(()=>shown('Teilen: fehlgeschlagen'),'"Teilen: fehlgeschlagen"');
  });
  await step('no navigator.share: no CAP_SHARE, "nicht möglich" at once',async()=>{
    await load('none');
    await el('Teilen').dispatchEvent('click');
    await until(()=>shown('Teilen: nicht möglich'),'"Teilen: nicht möglich"');
  });
} finally {
  await browser.close();
  server.close();
}
console.log(failed ? `share_web: ${failed} failed` : 'share_web: ok');
process.exit(failed ? 1 : 0);
