// The accessibility tree in a real browser (web/a11y.js, lib/plat/web_a11y.rs): the ARIA
// shadow tree of the androidGallery web build as Chromium's own accessibility tree shows it
// (CDP Accessibility.getFullAXTree; headings with their level), clicks, slider values, text
// and selections through the DOM as assistive technology does them, the DOM focus following
// Tab, a pointer giving the keyboard back to #input, ?a11y=0. Playwright
// (tools/tests/playwright.mjs, PLAYWRIGHT_MODULE)
// with chromiumOptions() (CHROME_PATH, the installed Chrome, else Playwright's Chromium), or
// an installed browser channel such as --channel=msedge.
//   python build_rs.py app androidGallery --target web
//   node apps/androidGallery/tests/a11y_web.mjs [build] [--channel=chrome|msedge] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const {chromium}=loadPlaywright();
const dir=resolve(build,'web','androidGallery');
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length ? path : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    const bytes=await readFile(file);
    response.writeHead(200,{'content-type':type,'cache-control':'no-store'}).end(bytes);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}/index.html`;
const channel=option('channel');
const browser=await chromium.launch({...(channel?{channel}:chromiumOptions()),headless:!args.includes('--headed')});
let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
try {
  const page=await browser.newPage({viewport:{width:420,height:900}});
  const errors=[];
  page.on('pageerror',e=>errors.push(String(e)));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  await page.goto(base);
  await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelectorAll('#a11y [data-key]').length>20);
  // the elements by their accessible name (aria-label, text) -- what assistive technology finds
  const el=name=>page.locator(`#a11y [aria-label="${name}"]`).first();
  const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
  const until=async(fn,what,ms=8000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`timed out: ${what}`);};
  const active=()=>page.evaluate(()=>{const a=document.activeElement;return a?.id==='input'?'#input':a?.getAttribute('aria-label') ?? a?.textContent ?? a?.tagName;});

  // Chromium's accessibility tree: the shadow tree is there (opacity 0 hides nothing), with roles, names, states, values
  const cdp=await page.context().newCDPSession(page);
  const axTree=async()=>{
    const {nodes}=await cdp.send('Accessibility.getFullAXTree');
    return nodes.filter(n=>!n.ignored).map(n=>({role:n.role?.value,name:n.name?.value ?? '',
      props:Object.fromEntries((n.properties || []).map(p=>[p.name,p.value?.value])),value:n.value?.value}));
  };
  // Chromium collapses the white space of names
  const find=(tree,role,name)=>tree.find(n=>n.role===role && (name===undefined || n.name===name.replace(/\s+/g,' ')));
  let ax=await axTree();
  ok(find(ax,'region','jfgui Pocket Gallery'),'the main window is a region named by its title');
  ok(find(ax,'button','Klick mich') && find(ax,'button','Reset'),'buttons');
  ok(find(ax,'checkbox','Option aktivieren')?.props.checked==='false','check box, unchecked');
  ok(find(ax,'switch','Regler aktiv')?.props.checked==='true','switch, on');
  const slider=find(ax,'slider','Wert: 40 %');
  ok(slider && Number(slider.value ?? slider.props.valuenow)===40,'slider with its name from the text before it and its value');
  ok(Number(find(ax,'progressbar')?.props.valuenow ?? find(ax,'progressbar')?.value)===40,'progress bar value');
  if(process.env.A11Y_DUMP) console.log(JSON.stringify(ax.filter(n=>!['generic','none'].includes(n.role)),null,0));
  ok(find(ax,'textbox','03  Texteingabe'),'text box named by the text above it');
  ok(ax.some(n=>n.role==='StaticText' && n.name==='Kleine Widgets.'),'labels are text');
  const heading=find(ax,'heading','01  Interaktion');
  ok(heading && Number(heading.props.level)===2,'a section title (ui.heading) is a heading of level 2');
  ok(!find(ax,'heading','Kleine Widgets.'),'a text is no heading');
  ok(await page.locator('#a11y [role=heading][aria-level="2"]').first().textContent()==='01  Interaktion','role=heading, aria-level in the shadow tree');
  ok(find(ax,'button','#0078D4'),'colour swatches');

  // clicks as a screen reader sends them (the overlay takes no pointer events)
  await el('Option aktivieren').dispatchEvent('click');
  await until(async()=>await el('Option aktivieren').getAttribute('aria-checked')==='true','check box toggled');
  checks++;
  for(let i=0;i<2;i++) await el('Klick mich').dispatchEvent('click');
  await until(async()=>await text('Klicks:')==='Klicks: 2','two clicks counted');
  checks++;
  // a value from assistive technology: the range input's own input event
  await page.evaluate(()=>{const r=document.querySelector('#a11y input[type=range]');r.value='73';r.dispatchEvent(new Event('input',{bubbles:true}));});
  await until(async()=>await text('Wert:')==='Wert: 73 %','slider value set');
  ok(await page.locator('#a11y [role=progressbar]').getAttribute('aria-valuenow')==='73','progress follows');
  ax=await axTree();
  ok(find(ax,'checkbox','Option aktivieren')?.props.checked==='true','Chromium sees the check box checked');

  // the keyboard: Tab moves the app's focus, the DOM focus follows into the shadow tree
  await page.locator('#input').focus();
  await page.keyboard.press('Tab');
  await until(async()=>await active()==='Klick mich','DOM focus on the first button');
  checks++;
  await page.keyboard.press('Space');
  await until(async()=>await text('Klicks:')==='Klicks: 3','Space on the focused button');
  checks++;
  ok(await active()==='Klick mich','focus stays');
  for(const expected of ['Reset','Option aktivieren','Regler aktiv','Wert: 73 %']) {
    await page.keyboard.press('Tab');
    await until(async()=>await active()===expected,`Tab to ${expected}`);
    checks++;
  }
  await page.keyboard.press('ArrowRight');
  await until(async()=>await text('Wert:')==='Wert: 74 %','arrow key on the slider');
  ok(await page.locator('#a11y input[type=range]').inputValue()==='74','range input follows');
  ok(await page.locator('#a11y [tabindex="0"]').count()===1,'one element in the Tab order');

  // text: the field gets the focus from the DOM, typing goes to the app and comes back as value and caret
  const field=page.locator('#a11y input[type=text]').first(); // "03 Texteingabe" (04 has the note)
  await field.focus();
  await until(async()=>(await page.evaluate(()=>{const s=window.jfguiA11y,t=s.mirror.active();return t?.focus;}))===await field.getAttribute('data-key'),'app focus on the text box');
  checks++;
  await page.keyboard.type('Jäkob');
  await until(async()=>await field.inputValue()==='Jäkob','typed text mirrored');
  ok(await field.evaluate(f=>[f.selectionStart,f.selectionEnd].join())==='5,5','caret after the text');
  await until(async()=>await text('Hallo')==='Hallo Jäkob!','the app got the text');
  checks++;
  await page.keyboard.press('Backspace');
  await until(async()=>await field.inputValue()==='Jäko','Backspace');
  checks++;
  await page.keyboard.press('Home');
  await until(async()=>await field.evaluate(f=>f.selectionStart===0 && f.selectionEnd===0),'Home moves the caret (the app moves it)');
  checks++;
  // a selection made in the DOM (screen reader / text navigation) goes to the app
  await field.evaluate(f=>f.setSelectionRange(1,3,'forward'));
  await until(async()=>page.evaluate(k=>{const n=window.jfguiA11y.mirror.wins.get(0).nodes.get(k);return n.anchor===1 && n.caret===3;},await field.getAttribute('data-key')),'DOM selection -> app');
  checks++;
  await page.keyboard.type('X');
  await until(async()=>await field.inputValue()==='JXo','typing replaces the selection');
  checks++;
  // a replacement (autocorrect, voice input) changes the value directly
  await page.evaluate(()=>{const s=window.jfguiA11y,send=s.send;window.acts=[];s.send=m=>{window.acts.push(m);send(m);};});
  await field.evaluate(f=>{f.value='Emoji 🙂';f.setSelectionRange(8,8);f.dispatchEvent(new InputEvent('input',{inputType:'insertReplacementText',bubbles:true}));});
  await until(async()=>await text('Hallo')==='Hallo Emoji 🙂!','replacement reached the app');
  await until(async()=>(await page.evaluate(k=>{const n=window.jfguiA11y.mirror.wins.get(0).nodes.get(k);return n.caret===7 && n.anchor===7;},await field.getAttribute('data-key'))),
    'the app caret after the emoji (7 characters)',3000).catch(async e=>{throw new Error(e.message+' '+JSON.stringify(await page.evaluate(k=>{const n=window.jfguiA11y.mirror.wins.get(0).nodes.get(k);return [n.caret,n.anchor,n.text,window.acts];},await field.getAttribute('data-key'))));});
  ok(await field.evaluate(f=>f.selectionStart===8 && f.selectionEnd===8),'caret in UTF-16 units after the emoji');

  // a pointer on the canvas gives the keyboard back to #input
  await page.mouse.click(200,150);
  await until(async()=>await active()==='#input','pointer: #input has the keyboard');
  checks++;
  ok(errors.length===0,'no page errors: '+errors.join('; '));

  // ?a11y=0: no shadow tree
  const off=await browser.newPage({viewport:{width:420,height:900}});
  await off.goto(base+'?a11y=0');
  await off.waitForFunction(()=>document.body.dataset.compute);
  await off.waitForTimeout(500);
  ok(await off.locator('#a11y [data-key]').count()===0,'?a11y=0 builds no tree');
  console.log(`a11y web: ${checks} checks passed (Chromium accessibility tree, clicks, slider, Tab focus, text, selection, pointer, ?a11y=0)`);
} finally {
  await browser.close();
  server.close();
}
