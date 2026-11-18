// (G7a) The browser host's pointer lock and key positions, driven by Playwright's input (the browser's own synthetic
// events: nothing reaches the system's pointer or keyboard).  apps/inputtest built for the browser:
//   python build_rs.py app inputtest --target web
//   node tools/tests/web_input.mjs build [--engine=chromium,firefox]
// Checks: a press in the field locks the canvas (requestPointerLock from the press), the motion while locked adds up in
// the app (movementX / movementY -> jfgui_motion), the keys carry KeyboardEvent.code as USB HID usages (KeyEvent::code),
// Esc lets the lock go and is no key of the app; the button "Zeiger fangen" locks at once (the press's activation still
// holds when the app asks).  What the browser does with a real mouse (unadjustedMovement, the cursor) the user checks by
// hand (docs/GUIDE.md "Pointer lock").
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from './playwright.mjs';

const root=resolve('.'), build=resolve(process.argv[2] || 'build');
const options=Object.fromEntries(process.argv.slice(3).filter(v=>v.startsWith('--')).map(v=>v.slice(2).split('=')));
const engines=(options.engine || 'chromium').split(',');
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve(root,'build/browser-engines');
const playwright=loadPlaywright();
const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.length===1) parts.push('index.html');
    const filename=resolve(build,'web',...parts);
    if(!filename.startsWith(resolve(build,'web')+sep)) throw new Error('Invalid path');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm'}[extname(filename)] || 'application/octet-stream';
    const bytes=await readFile(filename);
    response.writeHead(200,{'content-type':type,'cache-control':'no-store'}).end(bytes);
  } catch(error) {response.writeHead(404,{'content-type':'text/plain'}).end(String(error));}
});
await new Promise(done=>server.listen(0,'127.0.0.1',done));
const base=`http://127.0.0.1:${server.address().port}`;
let id=0;
async function texts(page) {
  const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method:'state/inspect',params:{domain:'ui',limit:200}});
  assert.ok(r && !r.error,JSON.stringify(r));
  return r.result.widgets;
}
async function until(page,predicate,description,timeout=10000) {
  const end=Date.now()+timeout;
  let last;
  do {
    last=await texts(page);
    if(predicate(last.map(w=>w.label||''))) return last;
    await new Promise(done=>setTimeout(done,80));
  } while(Date.now()<end);
  throw new Error(`Timed out: ${description}; last=${JSON.stringify(last.map(w=>w.label).filter(Boolean))}`);
}
const has=s=>labels=>labels.includes(s);
let failures=0;
for(const engine of engines) {
  const type=playwright[engine];
  const browser=await launchWasm(type,{headless:true,...(engine==='chromium'?chromiumOptions():{})});
  try {
    const page=await browser.newPage();
    page.on('console',m=>{if(m.type()==='error')console.log(`[${engine}] console: ${m.text()}`);});
    await page.goto(`${base}/inputtest/?agent=1`);
    await page.waitForFunction(()=>window.jfguiAgent && document.body?.dataset?.compute!==undefined,{timeout:45000});
    const widgets=await until(page,has('Zeiger: aus'),'the app is up');
    const field=widgets.find(w=>w.label==='Feld');
    assert.ok(field,'the field (canvas "Feld")');
    const c=await page.locator('#screen').boundingBox(), dpr=await page.evaluate(()=>devicePixelRatio);
    const at=(r,fx,fy)=>[c.x+(r[0]+r[2]*fx)/dpr,c.y+(r[1]+r[3]*fy)/dpr];
    // a press in the field: requestPointerLock from the press
    const [fx,fy]=at(field.rect,0.5,0.5);
    await page.mouse.click(fx,fy);
    await until(page,has('Zeiger: gefangen'),'locked after a press in the field');
    assert.equal(await page.evaluate(()=>document.pointerLockElement?.id),'screen','the canvas holds the pointer lock');
    // motion while locked
    await page.mouse.move(fx+40,fy+25,{steps:4});
    await page.mouse.move(fx+10,fy+30,{steps:2});
    const moved=await until(page,l=>l.some(s=>s.startsWith('Summe:') && s!=='Summe: 0 0'),'the motion adds up');
    console.log(`[${engine}] ${moved.map(w=>w.label).find(s=>s?.startsWith('Summe:'))} (motion of 10, 30 asked)`);
    // keys by their place
    await page.keyboard.press('KeyW');
    await page.keyboard.press('KeyA');
    await until(page,l=>l.some(s=>s.startsWith('Tasten: +KeyW -KeyW +KeyA -KeyA')),'KeyboardEvent.code as KeyEvent::code');
    // Esc: the lock goes, no key for the app
    await page.keyboard.press('Escape');
    await until(page,has('Zeiger: aus'),'Esc lets go');
    assert.equal(await page.evaluate(()=>document.pointerLockElement),null,'no pointer lock after Esc');
    const keys=(await texts(page)).map(w=>w.label||'').find(s=>s.startsWith('Tasten:'));
    assert.ok(!keys.includes('Escape'),`Esc is the lock's: ${keys}`);
    // the button: the app asks while the press's activation holds
    const button=(await texts(page)).find(w=>w.label==='Zeiger fangen');
    const [bx,by]=at(button.rect,0.5,0.5);
    await page.mouse.click(bx,by);
    await until(page,has('Zeiger: gefangen'),'locked by the button (the activation of its press)');
    const free=(await texts(page)).find(w=>w.label==='Freigeben');
    const [rx,ry]=at(free.rect,0.5,0.5);
    await page.mouse.click(rx,ry); // (locked: the click lands where the pointer stands, the button's press)
    await page.evaluate(()=>document.exitPointerLock());
    await until(page,has('Zeiger: aus'),'let go');
    console.log(`[${engine}] ok`);
  } catch(error) {
    failures++;
    console.log(`[${engine}] FAIL ${error.message}`);
  } finally {
    await browser.close();
  }
}
server.close();
process.exit(failures ? 1 : 0);
