// The template of lib/jflib/template, built through Cargo for the browser by tools/crates_accept.py:
// the page starts without an error, two clicks on "Klick mich" count, "Zurücksetzen" resets.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
//   node tools/tests/crates_web.mjs build/crates_accept/web/hallo [--engines=chromium,firefox] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from './playwright.mjs';

const args=process.argv.slice(2);
const dir=resolve(args.find(a=>!a.startsWith('--')) || 'build/crates_accept/web/hallo');
const engines=(args.find(a=>a.startsWith('--engines='))?.split('=')[1] ?? 'chromium').split(',');
const pw=loadPlaywright();
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
let checks=0;
try {
  for(const engine of engines) {
    const browser=await launchWasm(pw[engine],{...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed')});
    try {
      const page=await browser.newPage({locale:'de-DE',viewport:{width:480,height:320}});
      const errors=[];
      page.on('pageerror',e=>errors.push(String(e)));
      page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
      await page.goto(base);
      await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Klick mich"]'),null,{timeout:60000});
      const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
      const see=async want=>{
        const end=Date.now()+10000;
        while(await text('Klicks:')!==want) {
          if(Date.now()>end) throw new Error(`${engine}: "${want}" not shown (got "${await text('Klicks:')}")`);
          await page.waitForTimeout(40);
        }
        checks++;
      };
      const click=name=>page.locator(`#a11y [aria-label="${name}"]`).first().dispatchEvent('click');
      await see('Klicks: 0');
      await click('Klick mich');
      await click('Klick mich');
      await see('Klicks: 2');
      await click('Zurücksetzen');
      await see('Klicks: 0');
      assert.deepEqual(errors,[],`${engine}: errors on the page`);
      console.log(`${engine}: ok`);
    } finally {await browser.close();}
  }
} finally {server.close();}
console.log(`crates_web: ${checks} checks`);
