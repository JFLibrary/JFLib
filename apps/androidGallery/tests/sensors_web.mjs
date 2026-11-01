// The motion sensors and the location in a real browser (web/sensors.js; lib/plat/web_sensors.rs,
// sensors.rs) through the gallery's pages (apps/androidGallery/pg_sensors.rs), in Chromium and Firefox:
// - the location: nothing before the switch; the browser's permission (Playwright's grant) and
//   setGeolocation -> the fix on the page, live; refused -> "nicht erlaubt"; the permission taken back
//   and given again while the page is open (Chromium, CDP Browser.setPermission); a hidden tab pauses
//   it and it starts anew, asking nobody; leaving the page stops it;
// - the sensors: a browser without any says "nicht verfügbar"; Chromium's virtual sensors (CDP
//   Emulation.setSensorOverrideEnabled) and its orientation override (DeviceOrientation.
//   setDeviceOrientationOverride) reach the level, the compass and the phone's position; in Firefox
//   the same through dispatched DeviceMotionEvent / DeviceOrientationEvent objects; Safari's
//   requestPermission (a stand-in): asked, refused, wanting a gesture of its own;
// - only what a page needs runs (document.body.dataset.sensors), a hidden tab stops it.
//   Headless browsers keep every tab visible: the test sets document.hidden / visibilityState and
//   dispatches the event, as system_web.mjs does.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/sensors_web.mjs [build] [--engines=chromium,firefox] [--threads] [--headed]
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
const origin=`http://127.0.0.1:${server.address().port}`;
const base=`${origin}/index.html${threads?'?compute=threads':''}`;

// before the page's scripts: the visibility set by the test; Safari's question as a stand-in
function hooks({ask=null}={}) {
  let hidden=false;
  Object.defineProperty(document,'hidden',{configurable:true,get:()=>hidden});
  Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>hidden?'hidden':'visible'});
  window.__setHidden=h=>{hidden=h;document.dispatchEvent(new Event('visibilitychange'));};
  if(ask) {
    window.__asked=0;
    window.__answers=[...ask];
    const question=async()=>{
      window.__asked++;
      const a=window.__answers.shift()??'granted';
      if(a==='gesture') throw new DOMException('needs a gesture','NotAllowedError');
      return a;
    };
    window.DeviceMotionEvent.requestPermission=question;
    window.DeviceOrientationEvent.requestPermission=question;
  }
}

// German numbers on the page whatever the browser's own language is
const CONTEXT={viewport:{width:420,height:900},locale:'de-DE'};
let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const engines=option('engines','chromium,firefox').split(',');
try {
  for(const engine of engines) {
    const launch=prefs=>pw[engine].launch({...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed'),
      ...(prefs&&engine==='firefox'?{firefoxUserPrefs:prefs}:{})});
    const browser=await launch();
    try {
      const open=async(context,hookOptions)=>{
        await context.addInitScript(hooks,hookOptions);
        const page=await context.newPage();
        const errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
        await page.goto(base);
        await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Wasserwaage"]'));
        if(threads) assert.equal(await page.evaluate(()=>document.body.dataset.compute),'threads','the threads variant runs');
        const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
        const texts=()=>page.evaluate(()=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent));
        const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`timed out: ${what}`);};
        const text=start=>texts().then(t=>t.find(x=>x.startsWith(start)) ?? null);
        const see=async(want,what=want)=>{
          await until(async()=>(await texts()).includes(want),`${what}: "${want}" (texts: ${(await texts()).filter(t=>t).slice(0,20).join(' | ')})`);
          checks++;
        };
        const click=name=>el(name).dispatchEvent('click');
        const data=name=>page.evaluate(n=>document.body.dataset[n],name);
        const back=()=>el('Zurück').dispatchEvent('click');
        return {page,errors,el,texts,until,text,see,click,data,back};
      };
      // what the page's accessibility tree offers of a page: opened by the card's row
      const go=async(h,name)=>{await h.click(name);await h.until(async()=>h.el('Zurück').count(),`the page ${name}`);};

      // ---------------------------------------------------------------- the location
      {
        const context=await browser.newContext(CONTEXT);
        await context.grantPermissions(['geolocation'],{origin});
        await context.setGeolocation({latitude:48.137154,longitude:11.576124,accuracy:12});
        const h=await open(context);
        await go(h,'GPS');
        await h.see('Standort: aus','nothing before the switch');
        await h.see('Breite: –');
        ok(await h.data('location')==='off',`${engine}: the page does not watch yet`);
        await h.click('Standort verwenden');
        await h.see('Standort: Position da','the fix arrives');
        await h.see('Breite: 48,137154°');
        await h.see('Länge: 11,576124°');
        await h.see('Genauigkeit: 12 m');
        await h.see('Quelle: keine Angabe');
        ok(await h.data('location')==='fix',`${engine}: dataset.location = fix`);
        await h.until(async()=>/^Alter: \d+ s$/.test(await h.text('Alter:') ?? ''),'the age');
        await context.setGeolocation({latitude:-33.8688,longitude:151.2093,accuracy:30});
        await h.see('Breite: -33,868800°','the next fix: live');
        await h.see('Länge: 151,209300°');
        await h.see('Genauigkeit: 30 m');
        // a hidden tab: paused, nobody asks when it comes back
        await h.page.evaluate(()=>window.__setHidden(true));
        await h.see('Standort: pausiert','a hidden tab pauses it');
        ok(await h.data('location')==='off',`${engine}: the watch is gone while hidden`);
        await h.page.evaluate(()=>window.__setHidden(false));
        await h.see('Standort: Position da','visible again: a fix');
        // leaving the page stops it; back it runs again from the switch
        await h.back();
        await h.until(async()=>await h.data('location')==='off','the page closed: stopped');
        checks++;
        await go(h,'GPS');
        await h.see('Standort: Position da','back on the page: it runs again');
        await h.click('Standort verwenden');
        await h.see('Standort: aus','the switch off');
        await h.see('Breite: –');
        if(engine==='chromium') {
          // the permission taken back and given again while it runs
          await h.click('Standort verwenden');
          await h.see('Standort: Position da');
          const cdp=await browser.newBrowserCDPSession();
          const setting=async s=>{
            const blank=await context.newPage();
            const {targetInfo}=await (await context.newCDPSession(blank)).send('Target.getTargetInfo');
            await blank.close();
            await cdp.send('Browser.setPermission',{permission:{name:'geolocation'},setting:s,origin,browserContextId:targetInfo.browserContextId});
          };
          await setting('denied');
          await h.see('Standort: nicht erlaubt','the permission taken back');
          await setting('granted');
          await h.see('Standort: Position da','given again: the watch starts anew');
        }
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await context.close();
      }
      // refused: Chromium's headless prompt answers no; Firefox's through its testing preference
      {
        const refusing=engine==='firefox' ? await launch({'geo.prompt.testing':true,'geo.prompt.testing.allow':false}) : browser;
        const context=await refusing.newContext(CONTEXT);
        const h=await open(context);
        await go(h,'GPS');
        await h.click('Standort verwenden');
        await h.see('Standort: nicht erlaubt',`${engine}: the prompt answers no`);
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await context.close();
        if(refusing!==browser) await refusing.close();
      }

      // ---------------------------------------------------------------- no sensors
      {
        const context=await browser.newContext(CONTEXT);
        const h=await open(context);
        await go(h,'Wasserwaage');
        ok(await h.data('sensors')==='19',`${engine}: the level starts accelerometer, gravity, rotation: ${await h.data('sensors')}`);
        await h.see('Keine Daten');
        await h.see('Beschleunigung: nicht verfügbar','no events: not available');
        await h.see('Drehlage: nicht verfügbar');
        await h.back();
        await h.until(async()=>await h.data('sensors')==='0','stopped with the page');
        checks++;
        await go(h,'Kompass');
        await h.see('Magnetfeld: nicht verfügbar','the browser has no magnetic field');
        await h.see('Keine Richtung');
        await h.back();
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await context.close();
      }

      // ---------------------------------------------------------------- events as a phone's browser sends them
      {
        const context=await browser.newContext(CONTEXT);
        const h=await open(context);
        const motion=(x,y,z,lx=0,ly=0,lz=0,gyro=[0,0,0])=>h.page.evaluate(([x,y,z,lx,ly,lz,g])=>window.dispatchEvent(new DeviceMotionEvent('devicemotion',{
          accelerationIncludingGravity:{x,y,z},acceleration:{x:lx,y:ly,z:lz},rotationRate:{alpha:g[0],beta:g[1],gamma:g[2]},interval:16})),[x,y,z,lx,ly,lz,gyro]);
        const orient=(alpha,beta,gamma,absolute=false)=>h.page.evaluate(([alpha,beta,gamma,absolute])=>window.dispatchEvent(new DeviceOrientationEvent(absolute?'deviceorientationabsolute':'deviceorientation',{alpha,beta,gamma,absolute})),[alpha,beta,gamma,absolute]);
        await go(h,'Wasserwaage');
        await h.page.waitForTimeout(60);
        await motion(0,0,9.81);
        await h.see('Waagerecht','flat');
        await h.see('Beschleunigung: läuft');
        await h.see('0,00 0,00 9,81 m/s²');
        await h.page.waitForTimeout(60);
        await motion(1,2,9.5,0,0,0);
        await h.see('Quer: 5,9°   Längs: 11,8°','tilted: gravity from the two accelerations');
        await h.see('Geneigt');
        // hidden: nothing runs
        await h.page.evaluate(()=>window.__setHidden(true));
        await h.see('Schwerkraft: pausiert','hidden: paused');
        ok(await h.data('sensors')==='0',`${engine}: hidden: the page stops listening`);
        await h.page.evaluate(()=>window.__setHidden(false));
        await h.until(async()=>await h.data('sensors')==='19','visible: listening again');
        await h.back();
        await go(h,'Handylage');
        await h.page.waitForTimeout(60);
        await motion(0,0,9.81,0,0,0,[10,20,30]);
        await orient(90,0,0);
        await h.see('Gieren: 270,0° (relativ)','a quarter turn to the left, no north');
        await h.see('Nicken: 0,0°');
        await h.see('Rollen: 0,0°');
        await h.see('Drehrate: läuft');
        await h.page.waitForTimeout(60);
        await orient(0,90,0);
        await h.page.waitForTimeout(60);
        await motion(0,9.81,0);
        await h.see('Nicken: 90,0°','upright');
        await h.back();
        await go(h,'Kompass');
        await h.page.waitForTimeout(60);
        await orient(45,0,0,true);
        await h.see('315° NW','north from deviceorientationabsolute');
        await h.page.waitForTimeout(60);
        await orient(90,0,0);
        await h.see('315° NW','a relative event does not replace it');
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await h.back();
        await context.close();
      }

      // ---------------------------------------------------------------- Safari's question
      {
        const context=await browser.newContext(CONTEXT);
        const h=await open(context,{ask:['denied']});
        await go(h,'Wasserwaage');
        await h.see('Beschleunigung: nicht erlaubt','refused');
        ok(await h.page.evaluate(()=>window.__asked)>=1,`${engine}: asked`);
        await h.back();
        await h.page.evaluate(()=>{window.__answers=['gesture'];window.__asked=0;});
        await go(h,'Wasserwaage');
        // it wants a gesture of its own: the next tap asks
        await h.until(async()=>await h.page.evaluate(()=>window.__asked)===1,'asked from the page');
        await h.page.mouse.click(200,300);
        await h.until(async()=>await h.page.evaluate(()=>window.__asked)>=2,'asked again from the tap');
        await h.page.evaluate(()=>window.dispatchEvent(new DeviceMotionEvent('devicemotion',{accelerationIncludingGravity:{x:0,y:0,z:9.81},acceleration:{x:0,y:0,z:0}})));
        await h.see('Waagerecht','allowed after the tap: events count');
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await context.close();
      }

      // ---------------------------------------------------------------- Chromium's virtual sensors
      if(engine==='chromium') {
        const context=await browser.newContext(CONTEXT);
        await context.grantPermissions(['accelerometer','gyroscope','magnetometer'],{origin}).catch(()=>{});
        const h=await open(context);
        const cdp=await context.newCDPSession(h.page);
        let virtual=true;
        try {
          for(const type of ['accelerometer','linear-acceleration','gyroscope','absolute-orientation','relative-orientation','gravity'])
            await cdp.send('Emulation.setSensorOverrideEnabled',{enabled:true,type,metadata:{available:true,minimumFrequency:1,maximumFrequency:60}});
        } catch(error) {virtual=false;console.log(`chromium: no virtual sensors here (${error.message}): not checked`);}
        if(virtual) {
          const read=(type,reading)=>cdp.send('Emulation.setSensorOverrideReadings',{type,reading});
          await go(h,'Wasserwaage');
          await read('accelerometer',{xyz:{x:1,y:2,z:9.5}});
          await read('linear-acceleration',{xyz:{x:0,y:0,z:0}});
          await read('gyroscope',{xyz:{x:0,y:0,z:0}});
          await h.see('Quer: 5,9°   Längs: 11,8°','the virtual accelerometer reaches the level');
          await h.back();
          await go(h,'Kompass');
          await read('absolute-orientation',{quaternion:{x:0,y:0,z:0.3826834,w:0.9238795}});
          await read('relative-orientation',{quaternion:{x:0,y:0,z:0.3826834,w:0.9238795}});
          await h.see('315° NW','the virtual orientation reaches the compass');
          await h.back();
        }
        ok(h.errors.length===0,`${engine}: no page errors: ${h.errors.join('; ')}`);
        await context.close();
      }
      console.log(`${engine}: passed`);
    } finally {await browser.close();}
  }
  console.log(`sensors web: ${checks} checks passed (${engines.join(', ')}${threads?', threads':''})`);
} finally {server.close();}
