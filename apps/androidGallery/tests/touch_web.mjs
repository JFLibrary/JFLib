// The finger in the browser (web/loader.js, web/worker.js -> lib/ui/touch.rs) with Chromium's touch
// emulation (Input.dispatchTouchEvent, a phone's viewport, `pointer: coarse`): a tap clicks, the pan
// follows the finger 1:1, a quick swipe rolls on and a touch stops it, a slider drags sideways and a
// vertical swipe on it rolls the page, no scroll bars, a tool window by finger, the keyboard only for
// text fields (inputmode).  Positions from the ARIA shadow tree over the canvas.  Firefox has no touch
// emulation here: it is skipped.  Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/touch_web.mjs [build] [--engines=chromium] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
if(!option('engines','chromium').split(',').includes('chromium')) {
  console.log('touch web: skipped (the touch emulation needs Chromium)');
  process.exit(0);
}
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

let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const browser=await launchWasm(pw.chromium,{...chromiumOptions(),headless:!args.includes('--headed')});
try {
  const context=await browser.newContext({viewport:{width:412,height:860},deviceScaleFactor:2,hasTouch:true,isMobile:true});
  const page=await context.newPage();
  const errors=[];
  page.on('pageerror',e=>errors.push(String(e)));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  const cdp=await context.newCDPSession(page);
  const touch=(type,points)=>cdp.send('Input.dispatchTouchEvent',{type,touchPoints:points});
  const sleep=ms=>page.waitForTimeout(ms);
  const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await sleep(40);}while(Date.now()<end);throw new Error(`timed out: ${what}`);};
  // a finger from a to b in n steps, ms apart; rest: it rests before it lifts (no fling)
  const swipe=async([x0,y0],[x1,y1],{n=12,ms=16,rest=0}={})=>{
    await touch('touchStart',[{x:x0,y:y0}]);
    for(let i=1;i<=n;i++) {await touch('touchMove',[{x:x0+(x1-x0)*i/n,y:y0+(y1-y0)*i/n}]);await sleep(ms);}
    if(rest) await sleep(rest);
    await touch('touchEnd',[]);
  };
  const node=label=>page.locator(`#a11y [aria-label="${label}"]`).first();
  const has=label=>page.evaluate(l=>!!document.querySelector(`#a11y [aria-label="${l}"]`),label);
  const box=async label=>(await node(label).boundingBox());
  const tap=async label=>{const b=await box(label);await touch('touchStart',[{x:b.x+b.width/2,y:b.y+b.height/2}]);await touch('touchEnd',[]);};
  // the app's text with this start, from the mirror of its accessibility tree
  const text=start=>page.evaluate(s=>{for(const w of window.jfguiA11y.mirror.wins.values())for(const n of w.nodes.values())if(typeof n.name==='string'&&n.name.startsWith(s))return n.name;for(const w of window.jfguiA11y.mirror.wins.values())for(const n of w.nodes.values())if(typeof n.text==='string'&&n.text.startsWith(s))return n.text;return null;},start);
  const settle=async label=>{let a=await box(label),n=0;for(;n<100;n++){await sleep(80);const b=await box(label);if(b&&a&&b.y===a.y&&b.x===a.x)return b;a=b;}return a;};

  await page.goto(base);
  await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Klick mich"]'));
  ok(await page.evaluate(()=>matchMedia('(pointer: coarse)').matches),'a phone: pointer coarse');

  // no scroll bars from the first frame on: the right margin of the canvas stays one colour
  const margin=()=>page.evaluate(()=>{
    const c=document.querySelector('#screen'),g=c.getContext('2d'),set=new Set();
    const d=g.getImageData(c.width-16,0,8,c.height).data;
    for(let i=0;i<d.length;i+=4)set.add((d[i]<<16)|(d[i+1]<<8)|d[i+2]);
    return set.size;
  });
  ok(await margin()===1,'no scroll bar with a finger');

  // the keyboard: none for a touch outside a text field
  ok(await page.evaluate(()=>document.querySelector('#input').inputMode)==='none','no keyboard at the start');

  // a tap clicks
  await tap('Klick mich');
  await until(()=>text('Klicks: 1'),'a tap clicks');
  ok(true,'a tap clicks');
  ok(await page.evaluate(()=>document.querySelector('#input').inputMode)==='none','no keyboard after a tap on a button');

  // the pan 1:1
  const y0=(await box('Klick mich')).y;
  await swipe([206,600],[206,400],{n:20,ms:16,rest:200});
  const b1=await settle('Klick mich');
  ok(Math.abs(y0-b1.y-200)<=1,`a finger that rests before it lifts moves the page by its way: ${y0-b1.y} of 200`);
  ok(await text('Klicks: 1'),'the swipe is no click');
  await swipe([206,400],[206,600],{n:20,ms:16,rest:200});
  const b2=await settle('Klick mich');
  ok(Math.abs(b2.y-y0)<=1,'and back');

  // the fling: more than the finger's way, in frames with the display
  await swipe([206,700],[206,600],{n:5,ms:12});
  await sleep(120);
  const moving=await box('Klick mich');
  await sleep(120);
  const moving2=await box('Klick mich').catch(()=>null);
  ok(!moving||!moving2||moving.y!==moving2.y,'it rolls on after the finger lifted');
  await until(async()=>{const a=await page.evaluate(()=>document.querySelector('#a11y').innerHTML.length);await sleep(300);return a===await page.evaluate(()=>document.querySelector('#a11y').innerHTML.length);},'it comes to rest',8000);
  ok(!(await has('Klick mich'))||(await box('Klick mich')).y<y0-140,'further than the finger went');
  // a touch stops it
  await swipe([206,800],[206,700],{n:5,ms:10});
  await sleep(60);
  await touch('touchStart',[{x:400,y:500}]);
  await sleep(100);
  const held=await page.evaluate(()=>document.querySelector('#a11y').innerHTML);
  await sleep(400);
  ok(held===await page.evaluate(()=>document.querySelector('#a11y').innerHTML),'a touch stops it');
  await touch('touchEnd',[]);

  // the edge of the screen is the page's too
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  await settle('Klick mich');
  await swipe([409,600],[409,500],{n:12,ms:16,rest:200});
  ok(Math.abs(y0-(await settle('Klick mich')).y-100)<=1,'a swipe at the very edge of the screen rolls the page');

  // back to the start; the slider: sideways it drags, a vertical swipe on it rolls the page
  for(let i=0;i<8;i++) await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  const s0=await settle('Klick mich');
  ok(s0&&Math.abs(s0.y-y0)<=1,'back at the start');
  ok(await text('Wert: 40'),'the slider at 40');
  const below=s0.y+s0.height;
  // (the slider's row lies 150 to 200 px below the button)
  const sliderY=await page.evaluate(()=>{const e=document.querySelector('#a11y input[type=range]');const r=e.getBoundingClientRect();return r.y+r.height/2;});
  ok(sliderY>below,'the slider in the shadow tree');
  await swipe([120,sliderY],[280,sliderY],{n:10,ms:16,rest:100});
  const value=await until(async()=>{const t=await text('Wert: ');return t&&!t.startsWith('Wert: 40')&&t;},'the slider moved');
  ok(value,`a horizontal drag moves the slider: ${value}`);
  ok(Math.abs((await settle('Klick mich')).y-y0)<=1,'the page stays');
  await swipe([206,sliderY],[206,sliderY-120],{n:12,ms:16,rest:200});
  ok(await text('Wert: ')===value,'a vertical swipe on it leaves the value');
  ok(Math.abs(y0-(await settle('Klick mich')).y-120)<=1,'and rolls the page');
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});

  // a second finger takes nothing from the first; when the first lifts, the pan goes on with it
  const a0=(await settle('Klick mich')).y;
  await page.evaluate(()=>{document.querySelector('#screen').addEventListener('pointerdown',e=>{if(e.isPrimary)window.__first=e.pointerId;},true);});
  await touch('touchStart',[{id:0,x:206,y:600}]);
  await touch('touchMove',[{id:0,x:206,y:560}]);await sleep(30);
  await touch('touchMove',[{id:0,x:206,y:520}]);await sleep(30);
  await touch('touchStart',[{id:0,x:206,y:520},{id:1,x:300,y:700}]);await sleep(30);
  await touch('touchMove',[{id:0,x:206,y:500},{id:1,x:300,y:400}]);await sleep(60);
  ok(Math.abs(a0-(await box('Klick mich')).y-100)<=1,'a second finger moves nothing');
  // (the emulation lifts only all fingers at once: the first one's pointerup by hand)
  await page.evaluate(()=>document.querySelector('#screen').dispatchEvent(new PointerEvent('pointerup',{pointerId:window.__first,pointerType:'touch',isPrimary:true,clientX:206,clientY:500,bubbles:true})));
  await sleep(30);
  await touch('touchMove',[{id:1,x:300,y:380}]);await sleep(60);
  ok(Math.abs(a0-(await box('Klick mich')).y-120)<=1,'the first one lifted: the pan goes on with the other one');
  await sleep(200);
  await touch('touchEnd',[]);
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});

  // the keyboard comes with a text field
  await swipe([206,700],[206,400],{n:12,ms:16,rest:200});
  await settle('Klick mich');
  const field=page.locator('#a11y input[type=text]').first();
  const fb=await field.boundingBox();
  await touch('touchStart',[{x:fb.x+fb.width/2,y:fb.y+fb.height/2}]);await touch('touchEnd',[]);
  await until(()=>page.evaluate(()=>document.querySelector('#input').inputMode==='text'),'the keyboard for a text field');
  ok(true,'a tap on a text field: inputmode text');
  await tap('Klick mich');
  await until(()=>page.evaluate(()=>document.querySelector('#input').inputMode==='none'),'no keyboard elsewhere');
  await swipe([206,200],[206,800],{n:10,ms:10,rest:60});
  ok(true,'a tap elsewhere: inputmode none again');

  // a tool window by finger: the page "Fenster & Dialoge" from the menu
  await tap('Seiten');
  const item=page.locator('#a11y [role=menuitem][aria-label="Fenster & Dialoge"]');
  await until(()=>item.count(),'the menu');
  const ib=await item.boundingBox();
  await touch('touchStart',[{x:ib.x+ib.width/2,y:ib.y+ib.height/2}]);await touch('touchEnd',[]);
  await until(()=>has('Werkzeugfenster'),'the page');
  await tap('Werkzeugfenster');
  await until(()=>has('Werkzeuge'),'the tool window');
  const t1=await settle('Werkzeuge');
  await swipe([t1.x+40,t1.y+14],[t1.x+80,t1.y+114],{n:10,ms:16,rest:100});
  const t2=await settle('Werkzeuge');
  ok(Math.abs(t2.x-t1.x-40)<=1&&Math.abs(t2.y-t1.y-100)<=1,`the window follows the finger: ${t2.x-t1.x}, ${t2.y-t1.y}`);
  await swipe([t2.x+40,t2.y+14],[1,1],{n:10,ms:16,rest:100});
  const t3=await settle('Werkzeuge');
  ok(t3.y>=0&&t3.height>0,'not out of reach');

  assert.deepEqual(errors,[],'no errors in the page');
  console.log(`touch web: ${checks} checks passed (Chromium's touch emulation)`);
} finally {
  await browser.close();
  server.close();
}
