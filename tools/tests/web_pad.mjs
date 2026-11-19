// (G7a part 2) The browser host's game pads with a fake navigator.getGamepads (no real pad, no system input):
// apps/inputtest built for the browser:
//   python build_rs.py app inputtest --target web
//   node tools/tests/web_pad.mjs build [--engine=chromium,firefox]
// Checks: the app's first ui.gamepads starts the page's polling (service 33), a pad the page reports comes as
// PADEV_CONNECT with its id and style, its buttons (a trigger's value) and sticks reach ui.gamepads, the presses in
// order, nothing held while the page has no focus (document.hasFocus), a pad without the "standard" mapping is marked,
// one that goes is PADEV_DISCONNECT.  What a real pad does (the browser shows it only after a press) the user checks by
// hand (apps/inputtest/README.md).
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
  const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method:'state/inspect',params:{domain:'ui',limit:300}});
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
const lacks=s=>labels=>!labels.includes(s);
// the fake: window.__padSet(index, state|null) sets what navigator.getGamepads reports (and fires the event the host
// listens for), window.__padFocus what document.hasFocus says
function fake() {
  const pads=[];
  window.__padFocus=true;
  Document.prototype.hasFocus=function() {return window.__padFocus;};
  window.__padSet=(i,state)=>{
    pads[i]=state ? {index:i,connected:true,timestamp:performance.now(),mapping:'standard',...state} : null;
    window.dispatchEvent(new Event(state ? 'gamepadconnected' : 'gamepaddisconnected'));
  };
  Object.defineProperty(Navigator.prototype,'getGamepads',{value:function() {return pads.slice();},configurable:true});
}
const button=(value,pressed=value>0.5)=>({value,pressed});
function pad({held=[],values={},axes=[0,0,0,0],id='Xbox 360 Controller (XInput STANDARD GAMEPAD)',mapping='standard'}={}) {
  const buttons=Array.from({length:17},(_,i)=>held.includes(i) ? button(1,true) : button(values[i]||0,(values[i]||0)>30/255));
  return {id,mapping,buttons,axes};
}
let failures=0;
for(const engine of engines) {
  const type=playwright[engine];
  const browser=await launchWasm(type,{headless:true,...(engine==='chromium'?chromiumOptions():{})});
  try {
    const page=await browser.newPage();
    page.on('console',m=>{if(m.type()==='error')console.log(`[${engine}] console: ${m.text()}`);});
    await page.addInitScript(fake);
    await page.goto(`${base}/inputtest/?agent=1`);
    await page.waitForFunction(()=>window.jfguiAgent && document.body?.dataset?.compute!==undefined,{timeout:45000});
    await until(page,l=>l.some(s=>s.startsWith('Pads: keine')),'the app watches the pads, none yet');
    const set=(i,state)=>page.evaluate(([i,state])=>window.__padSet(i,state),[i,state]);
    // a pad comes with A held and the left stick moved
    await set(0,pad({held:[0],axes:[0.5,-0.25,0,0]}));
    await until(page,has('Pad 0: Xbox 360 Controller (XInput STANDARD GAMEPAD) · Xbox'),'the pad comes, its style from its id');
    await until(page,has('Pad 0 Knöpfe: A'),'A held');
    await until(page,has('Pad 0 Sticks: L 0.50 -0.25 · R 0.00 0.00'),'the left stick');
    await until(page,has('Pad-Ereignisse: +0'),'PADEV_CONNECT (UIC_GAMEPAD)');
    // the right trigger with its value, then everything let go
    await set(0,pad({held:[0],values:{7:0.75},axes:[0.5,-0.25,0,0]}));
    await until(page,has('Pad 0 Knöpfe: A RT 0.75'),'RT 0.75');
    await set(0,pad());
    await until(page,has('Pad 0 Knöpfe: –'),'let go');
    await until(page,has('Pad-Drücke: A RT'),'the presses in order');
    // the page without the focus: nothing held, the pad stays
    await set(0,pad({held:[1]}));
    await until(page,has('Pad 0 Knöpfe: B'),'B held');
    await page.evaluate(()=>{window.__padFocus=false;window.dispatchEvent(new Event('blur'));});
    await until(page,has('Pad 0 Knöpfe: –'),'no focus: nothing held');
    await page.evaluate(()=>{window.__padFocus=true;window.dispatchEvent(new Event('focus'));});
    await until(page,has('Pad 0 Knöpfe: B'),'the focus back: B again');
    // a pad without the standard mapping, then the first one goes
    await set(2,pad({id:'054c-05c4-Wireless Controller',mapping:'',held:[3]}));
    await until(page,has('Pad 2: 054c-05c4-Wireless Controller · PlayStation · eigene Belegung'),'not mapped, its style from the vendor');
    await set(0,null);
    await until(page,lacks('Pad 0: Xbox 360 Controller (XInput STANDARD GAMEPAD) · Xbox'),'pad 0 went');
    await until(page,has('Pad-Ereignisse: +0 +2 -0'),'PADEV_DISCONNECT');
    await set(2,null);
    await until(page,l=>l.some(s=>s.startsWith('Pads: keine')),'none left');
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
