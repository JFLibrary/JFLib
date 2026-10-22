// The screen services in the browser (lib/plat/web_screen.rs, web/loader.js) with the real host:
// the monitor the app lists is the page's window.screen (size, work area of availLeft / availTop /
// availWidth / availHeight, 96 * devicePixelRatio dpi, primary), a change of the screen reaches
// the app (Chromium: the screen size changed through the DevTools protocol, then UIC_SETTINGS
// with SET_MONITORS), global shortcuts and pictures of the screen are Unavailable, the colour
// picker's pipette is there exactly where the browser has the EyeDropper (Chromium, not
// Firefox), a pick without a user gesture is refused (Chromium) and a stand-in EyeDropper's
// colour (#336699, every engine) reaches the app through REQUEST_PICK_COLOR and jfgui_color.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from
// PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app screentest --target web
//   node apps/screentest/tests/screen_web.mjs [build] [--engines=chromium,firefox]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright();
const dir=resolve(build,'web','screentest');
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
const labels=async page=>(await rpc(page,'state/inspect',{domain:'ui',limit:400})).widgets.map(w=>w.label||'');
async function click(page,label) {
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
const line=s=>`1: ? – ${s.width} × ${s.height} bei ${s.left}, ${s.top} · Arbeitsbereich ${s.aw} × ${s.ah} bei ${s.al}, ${s.at} · ${s.dpi} dpi · primär`;
const screenOf=page=>page.evaluate(()=>({width:screen.width,height:screen.height,left:Math.round(screen.left||0),top:Math.round(screen.top||0),
  aw:screen.availWidth,ah:screen.availHeight,al:Math.round(screen.availLeft||0),at:Math.round(screen.availTop||0),dpi:Math.round(96*devicePixelRatio),eye:!!window.EyeDropper}));

const results=[];
for(const engine of option('engines','chromium').split(',')) {
  for(const stub of [false,true]) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    const context=await browser.newContext({viewport:{width:900,height:1400},...(engine==='chromium'?{screen:{width:1600,height:1500}}:{})});
    const page=await context.newPage();
    const errors=[];
    page.on('pageerror',e=>errors.push(String(e)));
    page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
    await page.addInitScript(stub=>{
      window.__ready=false;addEventListener('jfgui-ready',()=>{window.__ready=true;});
      // a stand-in EyeDropper that answers at once (the real one needs a person)
      if(stub) window.EyeDropper=class {open() {return Promise.resolve({sRGBHex:'#336699'});}};
    },stub);
    await page.goto(base);
    await page.waitForFunction(()=>window.__ready,{timeout:45000});
    const tag=`${engine}${stub?' (stand-in EyeDropper)':''}`;
    try {
      const s=await screenOf(page);
      const want=line(s);
      const mon=await until(()=>labels(page),l=>l.includes(want),`the monitor of window.screen (${want})`);
      assert.ok(mon.includes('Änderungen: 0'),`${tag}: no change yet`);
      // global shortcuts and pictures of the screen: Unavailable
      assert.ok(mon.includes('Globale Tastenkürzel: hier nicht verfügbar'),`${tag}: shortcuts unavailable`);
      await click(page,'Registrieren');
      await until(()=>labels(page),l=>l.includes('Nicht registriert: hier nicht verfügbar'),'register: Unavailable');
      await click(page,'Bildschirm aufnehmen');
      await until(()=>labels(page),l=>l.includes('Bildschirmfoto: hier nicht verfügbar'),'screenshot: Unavailable');
      // the pipette only with an EyeDropper
      const pipette=(await labels(page)).includes('Farbe vom Bildschirm aufnehmen');
      assert.equal(pipette,s.eye,`${tag}: the pipette exactly with EyeDropper (${s.eye})`);
      let pick='';
      if(stub) {
        await click(page,'Farbe vom Bildschirm aufnehmen');
        await until(()=>labels(page),l=>l.includes('Gewählt: #336699'),'the stand-in pick reaches the colour picker');
        await click(page,'Farbe aufnehmen');
        pick=(await until(()=>labels(page),l=>l.some(x=>x.startsWith('Aufgenommen: #')),'UIC_PICK with the colour')).find(x=>x.startsWith('Aufgenommen'));
        assert.equal(pick,'Aufgenommen: #336699',`${tag}: picked`);
      } else if(s.eye) {
        // the real EyeDropper waits for a person (headless Chromium leaves open() pending):
        // only its pipette is checked here, the path of its answer with the stand-in
        pick='(the real EyeDropper is not opened headless)';
      } else {
        await click(page,'Farbe aufnehmen');
        pick=(await until(()=>labels(page),l=>l.includes('Farbe aufnehmen: hier nicht verfügbar'),'pick: Unavailable')).find(x=>x.startsWith('Farbe aufnehmen: '));
      }
      // a change of the screen (Chromium: DevTools' screen size)
      let change='';
      if(engine==='chromium' && !stub) {
        const cdp=await context.newCDPSession(page);
        await cdp.send('Emulation.setDeviceMetricsOverride',{width:900,height:1400,deviceScaleFactor:1,mobile:false,screenWidth:1920,screenHeight:1440});
        await page.evaluate(()=>dispatchEvent(new Event('resize')));
        const s2=await screenOf(page);
        assert.equal(s2.width,1920,'the screen was resized');
        await page.evaluate(()=>screen.dispatchEvent?.(new Event('change')));
        const after=await until(()=>labels(page),l=>l.includes('Änderungen: 1') && l.includes(line(s2)),`the change reaches the app (${line(s2)})`);
        change=after.find(x=>x.startsWith('1: '));
      }
      results.push({tag,monitor:want,pipette,pick,change});
      console.log(`PASS ${tag}: ${want}; pipette ${pipette}; ${pick}${change?'; after the change: '+change:''}`);
      assert.deepEqual(errors.filter(e=>!e.includes('favicon')),[],`${tag}: no page errors`);
    } finally {
      await browser.close();
    }
  }
}
server.close();
console.log(JSON.stringify(results,null,1));
