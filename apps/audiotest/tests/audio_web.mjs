// Sound in the browser (lib/plat/web_audio.rs, web/audio.js) with the real host: the tone
// generator (a fill stream) and the chime (play_file through the mixer) reach the page's
// AudioContext -- an AnalyserNode on the stream's worklet node measures the 440 Hz tone
// (Goertzel against another frequency) -- with the SharedArrayBuffer ring (cross-origin
// isolated, the threads variant) and with posted buffers; stop closes the node; a suspended
// context (what browsers do before the first user gesture; Chrome headless allows autoplay
// whatever the policy flag says, so the test suspends it) shows State::Blocked ("Wartet auf
// einen Klick") until a real click resumes it. Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from
// PLAYWRIGHT_BROWSERS_PATH); a browser that runs no AudioContext headless is reported, not failed.
//   python build_rs.py app audiotest --target web
//   node apps/audiotest/tests/audio_web.mjs [build] [--engines=chromium,firefox]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const pw=loadPlaywright();
const dir=resolve(build,'web','audiotest');
let isolated=false;
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length ? path : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    const bytes=await readFile(file);
    const headers=isolated ? {'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'} : {};
    response.writeHead(200,{'content-type':type,'cache-control':'no-store',...headers}).end(bytes);
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
const widgets=async page=>(await rpc(page,'state/inspect',{domain:'ui',limit:300})).widgets;
// a click through the agent (no user gesture); the status line changes the revision while a
// stream plays, so a conflict is tried again
async function click(page,label) {
  for(let attempt=0;;attempt++) {
    const inspect=await rpc(page,'state/inspect',{domain:'ui',limit:300});
    const w=inspect.widgets.find(x=>x.label===label);
    assert.ok(w,`a widget "${label}"`);
    const r=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++id,method:'actions/execute',
      params:{action:'ui.click',input:{id:w.id},expectedRevision:inspect.revision}});
    if(!r?.error) return;
    assert.ok(r.error.data?.code==='revision_conflict' && attempt<20,`click ${label}: ${JSON.stringify(r)}`);
  }
}
const status=async page=>(await widgets(page)).map(w=>w.label||'').find(l=>l.includes('Latenz') || l==='Aus') || '';
// what the stream nodes play during ms: the highest level of freq and of another frequency
// (Goertzel, Hann window) and the peak over windows of 4096 samples taken every 40 ms (nodes
// that appear meanwhile are listened to as well)
async function listen(page,freq,ms=600) {
  return page.evaluate(async ([freq,ms])=>{
    const {context:ctx,nodes}=window.jfguiAudio;
    const a=ctx.createAnalyser();a.fftSize=4096;
    const mute=ctx.createGain();mute.gain.value=0;a.connect(mute).connect(ctx.destination);
    const tapped=new Set();
    const g=(x,f)=>{
      const w=2*Math.PI*f/ctx.sampleRate,c=2*Math.cos(w);let s1=0,s2=0,sum=0;
      for(let i=0;i<x.length;i++){const h=0.5-0.5*Math.cos(2*Math.PI*i/(x.length-1));sum+=h;const s=x[i]*h+c*s1-s2;s2=s1;s1=s;}
      return 2*Math.sqrt(Math.max(0,s1*s1+s2*s2-c*s1*s2))/sum;
    };
    const x=new Float32Array(a.fftSize);
    let at=0,off=0,peak=0;
    for(const end=performance.now()+ms;performance.now()<end;) {
      for(const n of nodes.values())if(!tapped.has(n)){n.connect(a);tapped.add(n);}
      await new Promise(r=>setTimeout(r,40));
      a.getFloatTimeDomainData(x);
      at=Math.max(at,g(x,freq));off=Math.max(off,g(x,freq*1.37));
      for(const v of x)peak=Math.max(peak,Math.abs(v));
    }
    for(const n of tapped)try{n.disconnect(a);}catch{}
    a.disconnect();
    return {state:ctx.state,rate:ctx.sampleRate,mode:window.jfguiAudio.mode,nodes:nodes.size,at,off,peak};
  },[freq,ms]);
}

const results=[];
for(const engine of option('engines','chromium').split(',')) {
  for(const mode of ['posted','ring','blocked']) {
    if(mode==='blocked' && engine!=='chromium') continue;
    isolated=mode==='ring';
    const flags=engine==='chromium' ? ['--autoplay-policy=no-user-gesture-required'] : [];
    const firefoxPrefs=engine==='firefox' ? {firefoxUserPrefs:{'media.autoplay.default':0,'media.autoplay.block-webaudio':false}} : {};
    const browser=await pw[engine].launch({headless:true,args:flags,...firefoxPrefs,...(engine==='chromium'?chromiumOptions():{})});
    const page=await (await browser.newContext({viewport:{width:900,height:820}})).newPage();
    const errors=[];
    page.on('pageerror',e=>errors.push(String(e)));
    page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
    await page.addInitScript(()=>{window.__ready=false;addEventListener('jfgui-ready',()=>{window.__ready=true;});});
    await page.goto(base);
    await page.waitForFunction(()=>window.__ready,{timeout:45000});
    const tag=`${engine}/${mode}`;
    try {
      await click(page,'Abspielen');
      await page.waitForFunction(()=>window.jfguiAudio?.nodes.size>0,{timeout:10000});
      if(mode==='blocked') {
        await until(()=>status(page),s=>s.startsWith('Spielt'),'the generator plays');
        await page.evaluate(()=>window.jfguiAudio.context.suspend());
        const blocked=await until(()=>status(page),s=>s.includes('Wartet auf einen Klick'),'State::Blocked while suspended');
        assert.equal(await page.evaluate(()=>window.jfguiAudio.context.state),'suspended');
        const box=await page.locator('#screen').boundingBox();
        await page.mouse.click(box.x+box.width-20,box.y+box.height-20); // an empty spot: a gesture
        const after=await until(()=>status(page),s=>s.startsWith('Spielt'),'playing after the click');
        results.push({tag,blocked,after});
      }
      const st=await until(()=>status(page),s=>s.startsWith('Spielt'),'the generator plays');
      const heard=await until(()=>listen(page,440),v=>v.state!=='running' || v.at>0.05,'the tone arrives',8000);
      if(heard.state!=='running') {
        results.push({tag,skipped:`the AudioContext does not run headless here (${heard.state})`});
        console.log(`SKIP ${tag}: the AudioContext does not run (${heard.state})`);
        await browser.close();
        continue;
      }
      assert.ok(heard.at>0.15 && heard.at<0.25 && heard.at>10*heard.off,`${tag}: 440 Hz at 0.2 (${JSON.stringify(heard)})`);
      assert.equal(heard.mode,mode==='ring' ? 'ring' : 'posted',`${tag}: the transport`);
      if(mode==='ring') assert.equal(await page.evaluate(()=>crossOriginIsolated),true);
      // a higher frequency through the slider's number box is not needed: the stop closes the node
      await click(page,'Anhalten');
      await until(()=>page.evaluate(()=>window.jfguiAudio.nodes.size),n=>n===0,'stop closes the worklet node');
      // the chime: play_file through the mixer
      await click(page,'Glocke (WAV)');
      const chime=await listen(page,880,1500);
      assert.ok(chime.peak>0.05,`${tag}: the chime arrives (${JSON.stringify(chime)})`);
      assert.ok(chime.at>0.01,`${tag}: the chime's 880 Hz (${JSON.stringify(chime)})`);
      const sound=await until(async()=>(await widgets(page)).map(w=>w.label||'').find(l=>l.includes('Glocke wird abgespielt')),Boolean,'the note');
      // the same chime as Ogg Vorbis (decoded in WASM)
      await click(page,'Glocke (Ogg Vorbis)');
      const vorbis=await listen(page,880,1500);
      assert.ok(vorbis.peak>0.05 && vorbis.at>0.01,`${tag}: the Ogg Vorbis chime arrives (${JSON.stringify(vorbis)})`);
      results.push({tag,status:st,tone:heard,chime:{at:chime.at,peak:chime.peak},sound});
      console.log(`PASS ${tag}: 440 Hz ${heard.at.toFixed(3)} (off ${heard.off.toFixed(4)}), ${heard.rate} Hz, ${heard.mode}; chime peak ${chime.peak.toFixed(3)}; ${st}`);
      assert.deepEqual(errors.filter(e=>!e.includes('favicon')),[],`${tag}: no page errors`);
    } finally {
      await browser.close();
    }
  }
}
server.close();
console.log(JSON.stringify(results,null,1));
