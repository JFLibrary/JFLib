// Full screen in the browser (lib/plat/web_windows.rs, web/loader.js, web/worker.js) with the
// real host: a real click on "Vollbild" (the user's gesture) makes the app ask for
// window_fullscreen(0, true), the page goes full screen through the Fullscreen API
// (REQUEST_FULLSCREEN) and the app shows it; the browser leaving full screen (as its Esc does:
// document.exitFullscreen) reaches the app as UIC_FULLSCREEN; a request without a gesture (the
// agent's click) is refused by the browser and the app's state goes back (UIC_FULLSCREEN).
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from
// PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app desktest --target web
//   node apps/desktest/tests/fullscreen_web.mjs [build] [--engines=chromium,firefox]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright();
const dir=resolve(build,'web','desktest');
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
const base=`http://127.0.0.1:${server.address().port}/index.html?agent=1`;

let id=0;
async function rpc(page,method,params={}) {
  const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method,params});
  assert.ok(r && !r.error,`${method}: ${JSON.stringify(r)}`);
  return r.result;
}
async function until(fn,ok,what,ms=10000) {
  const end=Date.now()+ms;
  let v;
  do {v=await fn();if(ok(v))return v;await new Promise(r=>setTimeout(r,100));} while(Date.now()<end);
  throw new Error(`timed out: ${what}; last ${JSON.stringify(v)}`);
}
const widgets=async page=>(await rpc(page,'state/inspect',{domain:'ui',limit:400})).widgets;
const status=async page=>(await widgets(page)).map(w=>w.label||'').find(x=>x.startsWith('Vollbild: '));
const inFull=page=>page.evaluate(()=>!!document.fullscreenElement);
// the agent's click (no gesture of the user)
async function agentClick(page,label) {
  for(let attempt=0;;attempt++) {
    const inspect=await rpc(page,'state/inspect',{domain:'ui',limit:400});
    const w=inspect.widgets.find(x=>x.label===label);
    assert.ok(w,`a widget "${label}"`);
    const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method:'actions/execute',
      params:{action:'ui.click',input:{id:w.id},expectedRevision:inspect.revision}});
    if(!r?.error) return;
    assert.ok(r.error.data?.code==='revision_conflict' && attempt<20,`click ${label}: ${JSON.stringify(r)}`);
  }
}
// a real click of the pointer on a widget (canvas px over devicePixelRatio)
async function userClick(page,label) {
  const w=(await widgets(page)).find(x=>x.label===label);
  assert.ok(w && w.rect,`a widget "${label}" with its rectangle`);
  const box=await page.locator('canvas').first().boundingBox();
  const dpr=await page.evaluate(()=>devicePixelRatio);
  await page.mouse.click(box.x+(w.rect[0]+w.rect[2]/2)/dpr,box.y+(w.rect[1]+w.rect[3]/2)/dpr);
}

const results=[];
for(const engine of option('engines','chromium').split(',')) {
  const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
  const page=await (await browser.newContext({viewport:{width:1000,height:700}})).newPage();
  const errors=[];
  page.on('pageerror',e=>errors.push(String(e)));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  await page.addInitScript(()=>{window.__ready=false;addEventListener('jfgui-ready',()=>{window.__ready=true;});});
  try {
    await page.goto(base);
    await page.waitForFunction(()=>window.__ready,{timeout:45000});
    await until(()=>status(page),s=>s==='Vollbild: Haupt aus | Werkzeug aus | Meldungen 0','the app windowed');
    // the user's click: full screen
    await userClick(page,'Vollbild');
    await until(()=>inFull(page),v=>v,'the page in full screen');
    await until(()=>status(page),s=>s==='Vollbild: Haupt an | Werkzeug aus | Meldungen 0','the app shows full screen, no UIC_FULLSCREEN');
    // the browser leaves it (as its Esc does): the app hears it
    await page.evaluate(()=>document.exitFullscreen());
    await until(()=>inFull(page),v=>!v,'the page windowed');
    await until(()=>status(page),s=>s==='Vollbild: Haupt aus | Werkzeug aus | Meldungen 1','UIC_FULLSCREEN after the browser left');
    // without a gesture (Chromium: the agent's click evaluated without one -- Playwright's own
    // evaluate counts as a gesture): the browser refuses, the app's state goes back
    let refused='(Chromium only)';
    if(engine==='chromium') {
      // (every evaluate of Playwright is a gesture: from here on the page is only asked
      // through the DevTools protocol without one, after the last gesture expired)
      const cdp=await page.context().newCDPSession(page);
      const quiet=async expression=>{
        const r=await cdp.send('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true,userGesture:false});
        assert.ok(!r.exceptionDetails,`evaluate: ${JSON.stringify(r)}`);
        return r.result.value;
      };
      const ask=request=>quiet(`window.jfguiAgent.request(${JSON.stringify({jsonrpc:'2.0',id:++id,...request})})`);
      await new Promise(r=>setTimeout(r,6000));
      const inspect=(await ask({method:'state/inspect',params:{domain:'ui',limit:400}})).result;
      const w=inspect.widgets.find(x=>x.label==='Vollbild');
      const r=await ask({method:'actions/execute',params:{action:'ui.click',input:{id:w.id},expectedRevision:inspect.revision}});
      assert.ok(!r?.error,`the agent's click: ${JSON.stringify(r)}`);
      const quietStatus=async()=>(await ask({method:'state/inspect',params:{domain:'ui',limit:400}})).result.widgets.map(x=>x.label||'').find(x=>x.startsWith('Vollbild: '));
      refused=await until(quietStatus,s=>s==='Vollbild: Haupt aus | Werkzeug aus | Meldungen 2','refused: back, UIC_FULLSCREEN');
      assert.equal(await quiet('!!document.fullscreenElement'),false,'refused: the page stays windowed');
    } else {
      await agentClick(page,'Vollbild');
      await until(()=>inFull(page),v=>v,'full screen again');
    }
    results.push({engine,refused});
    console.log(`PASS ${engine}: full screen from the click, left by the browser${engine==='chromium'?', refused without a gesture':''}`);
    assert.deepEqual(errors.filter(e=>!e.includes('favicon')),[],`${engine}: no page errors`);
  } finally {
    await browser.close();
  }
}
server.close();
console.log(JSON.stringify(results,null,1));
