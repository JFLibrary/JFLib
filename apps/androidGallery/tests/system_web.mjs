// The system in a real browser (web/system.js, web/notify.js; lib/plat/web_system.rs,
// web_notify.rs, web_power.rs) through the gallery's card "06 System" (apps/androidGallery/
// system.rs), in Chromium and Firefox:
// - settings live: emulated prefers-color-scheme, prefers-reduced-motion, prefers-contrast and
//   forced-colors (with the scheme's system colours) reach the app as UIC_SETTINGS with their
//   SET_* bits, the theme follows the system once the app asks for it ("Design des Systems"),
//   and the default back again;
// - notifications: granted (Playwright's permission) the page shows a real Notification with the
//   app's title and body; a click on it reports UIC_NOTE, closing it UIC_NOTE_GONE; not yet asked
//   (Chromium: the headless prompt answers "denied") the page asks from the click and the note
//   goes away refused; denied (CDP Browser.setPermission / Firefox's pref) nothing is sent and
//   ui.notify answers 0.  Headless Firefox cannot display a notification (its error event); the
//   test swallows that event so the click path runs on the real Notification object;
// - the screen kept on: the switch takes the Screen Wake Lock (Chromium: permission through
//   CDP), a hidden tab loses it (as the browser drops it) and gets it back once visible, the
//   switch off releases it; without the API the switch answers "nicht möglich";
// - the lifecycle: visibilitychange -> UIC_SUSPEND 2 / 0.  Headless browsers keep every tab
//   visible (bringToFront, minimizing and a second tab change nothing), so the test sets
//   document.hidden / visibilityState and dispatches the event as the browser would.
// Playwright (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE; Firefox from PLAYWRIGHT_BROWSERS_PATH).
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/system_web.mjs [build] [--engines=chromium,firefox] [--threads] [--headed]
// --threads: cross-origin isolated, the shared-memory WASM variant (?compute=threads)
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from '../../../tools/tests/playwright.mjs';

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

// before the page's scripts: the notifications and wake locks the page makes (to click / drop
// them as the system would), the visibility set by the test (headless keeps every tab visible)
function hooks({quietErrors=false,noWakeLock=false}={}) {
  window.__notes=[];
  if(window.Notification) {
    const N=window.Notification;
    class Recorded extends N {
      constructor(title,options) {
        super(title,options);
        if(quietErrors) this.addEventListener('error',e=>e.stopImmediatePropagation());
        window.__notes.push({title,body:options?.body,icon:options?.icon,note:this});
      }
    }
    Object.defineProperty(Recorded,'permission',{get:()=>N.permission});
    Recorded.requestPermission=(...a)=>N.requestPermission(...a);
    window.Notification=Recorded;
  }
  window.__locks=[];
  if(noWakeLock) Object.defineProperty(Navigator.prototype,'wakeLock',{configurable:true,get:()=>undefined});
  else if(navigator.wakeLock) {
    const request=navigator.wakeLock.request.bind(navigator.wakeLock);
    navigator.wakeLock.request=async type=>{const lock=await request(type);window.__locks.push(lock);return lock;};
  }
  let hidden=false;
  Object.defineProperty(document,'hidden',{configurable:true,get:()=>hidden});
  Object.defineProperty(document,'visibilityState',{configurable:true,get:()=>hidden?'hidden':'visible'});
  window.__setHidden=async h=>{
    hidden=h;
    // the browser drops a screen wake lock of a hidden page
    if(h) for(const l of window.__locks) if(!l.released) await l.release();
    document.dispatchEvent(new Event('visibilitychange'));
  };
}

let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
const engines=option('engines','chromium,firefox').split(',');
try {
  for(const engine of engines) {
    const launch=prefs=>launchWasm(pw[engine],{...(engine==='chromium'?chromiumOptions():{}),headless:!args.includes('--headed'),
      ...(prefs&&engine==='firefox'?{firefoxUserPrefs:prefs}:{})});
    const browser=await launch();
    let cdp=null;
    // (Chromium) a permission of the context's origin, as the user would set it
    const permission=async(context,name,setting)=>{
      cdp||=await browser.newBrowserCDPSession();
      const blank=await context.newPage();
      const {targetInfo}=await (await context.newCDPSession(blank)).send('Target.getTargetInfo');
      await blank.close();
      await cdp.send('Browser.setPermission',{permission:{name},setting,origin,browserContextId:targetInfo.browserContextId});
    };
    try {
      // one page of the gallery in a context: its helpers
      const open=async(context,hookOptions)=>{
        await context.addInitScript(hooks,hookOptions);
        const page=await context.newPage();
        const errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
        await page.goto(base);
        await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelector('#a11y [aria-label="Design des Systems"]'));
        if(threads) assert.equal(await page.evaluate(()=>document.body.dataset.compute),'threads','the threads variant runs');
        const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
        const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
        const until=async(fn,what,ms=10000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`${engine}: timed out: ${what}`);};
        const see=async(want,what)=>{
          const start=want.split(':')[0]+':';
          await until(async()=>(await text(start))===want,`${what}: "${want}" (got "${await text(start)}")`);
          checks++;
        };
        // the text starting with prefix contains part (the app's focus is the page's: not checked)
        const has=async(prefix,part,what)=>{
          await until(async()=>(await text(prefix))?.includes(part),`${what}: "${prefix}..${part}" (got "${await text(prefix)}")`);
          checks++;
        };
        const click=name=>el(name).dispatchEvent('click');
        return {page,errors,el,text,until,see,has,click};
      };

      // ---------------------------------------------------------------- settings
      {
        const context=await browser.newContext({viewport:{width:420,height:900}});
        const {page,errors,click,see,text}=await open(context);
        await see('Design: hell (System: hell)','the light scheme at the start');
        await see('Kontrast: normal  Bewegung: normal','no contrast, animations on');
        await see('Text: 100 %  Akzent: keiner','text 100 %, no accent');
        await see('Einstellungen: 0 Änderungen (zuletzt 0x0), Design 0','nothing changed yet');
        await page.emulateMedia({colorScheme:'dark'});
        await see('Design: hell (System: dunkel)','the system turns dark; the gallery stays light (AF_LIGHT)');
        await see('Einstellungen: 1 Änderungen (zuletzt 0x1), Design 0','UIC_SETTINGS with SET_THEME');
        await click('Design des Systems');
        await see('Design: dunkel (System: dunkel)','the theme follows the system');
        await page.emulateMedia({colorScheme:'light'});
        await see('Design: hell (System: hell)','... live back to light');
        ok((await text('Einstellungen:')).startsWith('Einstellungen: 2 Änderungen (zuletzt 0x1), Design'),`${engine}: two changes, SET_THEME`);
        await page.emulateMedia({reducedMotion:'reduce'});
        await see('Kontrast: normal  Bewegung: reduziert','prefers-reduced-motion: reduce');
        ok((await text('Einstellungen:')).includes('(zuletzt 0x10)'),'SET_OTHER');
        await page.emulateMedia({reducedMotion:'no-preference'});
        await see('Kontrast: normal  Bewegung: normal','animations on again');
        // Playwright's Firefox matches the emulated contrast at once but fires its change event
        // only at a later emulation: there the test hands the page the next update itself (a
        // resize event, after which the page reads every query again)
        const nudge=async more=>{
          if(engine!=='firefox') return true;
          const end=Date.now()+3000;
          while(await page.evaluate(()=>matchMedia('(prefers-contrast: more)').matches)!==more) {
            if(Date.now()>end) return false;
            await page.waitForTimeout(50);
          }
          await page.evaluate(()=>dispatchEvent(new Event('resize')));
          return true;
        };
        await page.emulateMedia({contrast:'more'});
        if(await nudge(true)) {
          await see('Kontrast: hoch  Bewegung: normal','prefers-contrast: more');
          ok((await text('Einstellungen:')).includes('(zuletzt 0x4)'),'SET_CONTRAST');
        } else console.log(`${engine}: Playwright's emulation of prefers-contrast did not apply here: not checked`);
        await page.emulateMedia({contrast:'no-preference'});
        await nudge(false);
        await see('Kontrast: normal  Bewegung: normal','normal contrast again');
        await page.emulateMedia({forcedColors:'active'});
        await see('Kontrast: hoch (Systemfarben)  Bewegung: normal','forced-colors: active with the system colours');
        await page.emulateMedia({forcedColors:'none'});
        await see('Kontrast: normal  Bewegung: normal','no forced colours');
        ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
        await context.close();
      }

      // ---------------------------------------------------------------- notifications
      {
        const context=await browser.newContext({viewport:{width:420,height:900}});
        await context.grantPermissions(['notifications'],{origin});
        const {page,errors,click,see}=await open(context,{quietErrors:engine==='firefox'});
        await see('Benachrichtigung: noch keine','none yet');
        await click('Benachrichtigung');
        await see('Benachrichtigung: #1 gezeigt','granted: shown');
        const first=await page.evaluate(()=>window.__notes.map(n=>({title:n.title,body:n.body})));
        ok(first.length===1 && first[0].title==='jfgui Galerie' && first[0].body==='Benachrichtigung Nummer 1',`${engine}: a real Notification with the app's title and body: ${JSON.stringify(first)}`);
        await page.evaluate(()=>window.__notes[0].note.dispatchEvent(new Event('click')));
        await see('Benachrichtigung: #1 geklickt','the click reaches the app (UIC_NOTE)');
        await click('Benachrichtigung');
        await see('Benachrichtigung: #2 gezeigt','a second one');
        // Chromium closes the shown notification; Firefox (nothing shown headless) gets the event
        // the system would send
        if(engine==='firefox') await page.evaluate(()=>window.__notes[1].note.dispatchEvent(new Event('close')));
        else await page.evaluate(()=>window.__notes[1].note.close());
        await see('Benachrichtigung: #2 weg','closed without a click (UIC_NOTE_GONE)');
        ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
        await context.close();
      }
      if(engine==='chromium') {
        // not asked yet: the page asks from the click (headless Chromium answers "denied")
        const context=await browser.newContext({viewport:{width:420,height:900}});
        const {page,click,see}=await open(context);
        ok(await page.evaluate(()=>Notification.permission)==='default','not asked yet');
        await click('Benachrichtigung');
        await see('Benachrichtigung: #1 weg','asked, refused: gone');
        ok(await page.evaluate(()=>window.__notes.length)===0,'nothing shown');
        await click('Benachrichtigung');
        await see('Benachrichtigung: nicht gezeigt','refused once: ui.notify answers 0');
        await context.close();
      }
      {
        // denied before: nothing reaches the page
        const denied=engine==='firefox' ? await launch({'permissions.default.desktop-notification':2}) : browser;
        const context=await denied.newContext({viewport:{width:420,height:900}});
        if(engine==='chromium') await permission(context,'notifications','denied');
        const {page,click,see,errors}=await open(context);
        ok(await page.evaluate(()=>Notification.permission)==='denied',`${engine}: denied`);
        await click('Benachrichtigung');
        await see('Benachrichtigung: nicht gezeigt','denied: ui.notify answers 0');
        ok(await page.evaluate(()=>window.__notes.length)===0,'nothing shown');
        ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
        await context.close();
        if(denied!==browser) await denied.close();
      }

      // ---------------------------------------------------------------- the screen kept on, the lifecycle
      {
        const context=await browser.newContext({viewport:{width:420,height:900}});
        if(engine==='chromium') await permission(context,'screen-wake-lock','granted');
        const {page,click,see,has,until,errors}=await open(context);
        const lock=()=>page.evaluate(()=>document.body.dataset.wakeLock||'');
        await see('Bildschirm an: nein','not held at the start');
        await has('Lebenszyklus:','im Vordergrund, Hintergrund 0×, zurück 0×','in the foreground');
        await click('Bildschirm anlassen');
        await see('Bildschirm an: ja','the switch holds INHIBIT_IDLE');
        await until(async()=>await lock()==='held','the page holds the Screen Wake Lock');
        ok(await page.evaluate(()=>window.__locks.length===1 && !window.__locks[0].released),`${engine}: one wake lock, held`);
        await page.evaluate(()=>window.__setHidden(true));
        await has('Lebenszyklus:','im Hintergrund, Hintergrund 1×, zurück 0×','hidden: UIC_SUSPEND 2');
        await until(async()=>await lock()==='waiting','hidden: the browser dropped the lock, the page waits');
        checks++;
        await page.evaluate(()=>window.__setHidden(false));
        await has('Lebenszyklus:','im Vordergrund, Hintergrund 1×, zurück 1×','visible again: UIC_SUSPEND 0');
        await until(async()=>await lock()==='held','visible again: the lock is taken again');
        ok(await page.evaluate(()=>window.__locks.length===2 && !window.__locks[1].released),`${engine}: a new wake lock after visibilitychange`);
        await click('Bildschirm anlassen');
        await see('Bildschirm an: nein','the switch off drops the guard');
        await until(async()=>await lock()==='released','... and the page releases the lock');
        ok(await page.evaluate(()=>window.__locks.every(l=>l.released)),`${engine}: every wake lock released`);
        ok(errors.length===0,`${engine}: no page errors: ${errors.join('; ')}`);
        await context.close();
      }
      {
        // a browser without the Screen Wake Lock: not possible
        const context=await browser.newContext({viewport:{width:420,height:900}});
        const {click,see}=await open(context,{noWakeLock:true});
        await click('Bildschirm anlassen');
        await see('Bildschirm an: nein (nicht möglich)','without navigator.wakeLock plat::inhibit answers None');
        await context.close();
      }
      console.log(`system web (${engine}${threads?', threads':''}): passed`);
    } finally {await browser.close();}
  }
  console.log(`system web: ${checks} checks passed (${engines.join(', ')}: scheme, theme following, reduced motion, contrast, forced colours; notifications granted / asked / denied with click and close; the Screen Wake Lock across hidden and visible; UIC_SUSPEND from visibilitychange)`);
} finally {
  server.close();
}
