// A web page's accessibility tree in a real browser (web/a11y.js with the rich format, a11y_wire
// VERSION2; lib/ui/a11y.rs, section 19 of the guide): the ARIA shadow tree of the a11ytest web
// build as Chromium's own accessibility tree shows it (CDP Accessibility.getFullAXTree) -- tables
// with headers, landmarks, a tree with levels, a radio group, a tool bar, a tooltip, a separator,
// live regions, a meter, a spin button, a scroll bar, an article and a figure out of view,
// descriptions, required / invalid --, then the requests that reach the page through the DOM
// (click, focus, a number, a focus that scrolls a node into view) and an announcement
// (ui.announce: the shadow tree's live region). Playwright (tools/tests/playwright.mjs,
// PLAYWRIGHT_MODULE) with chromiumOptions(), or --channel=msedge.
//   python build_rs.py app a11ytest --target web
//   node apps/a11ytest/tests/a11y_web.mjs [build] [--channel=chrome|msedge] [--headed]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const {chromium}=loadPlaywright();
const dir=resolve(build,'web','a11ytest');
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(path.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(path.length ? path : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    const bytes=await readFile(file);
    // (the threaded build needs cross-origin isolation)
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin',
      'cross-origin-embedder-policy':'require-corp'}).end(bytes);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}/index.html`;
const channel=option('channel');
const browser=await chromium.launch({...(channel?{channel}:chromiumOptions()),headless:!args.includes('--headed')});
let checks=0;
const ok=(cond,what)=>{assert.ok(cond,what);checks++;};
try {
  const page=await browser.newPage({locale:'de-DE',viewport:{width:600,height:700}});
  const errors=[];
  page.on('pageerror',e=>errors.push(String(e)));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  await page.goto(base);
  await page.waitForFunction(()=>window.jfguiA11y?.updates>0 && document.querySelectorAll('#a11y [role=grid]').length>0,null,{timeout:60000});
  const until=async(fn,what,ms=8000)=>{const end=Date.now()+ms;let v;do{v=await fn();if(v)return v;await page.waitForTimeout(40);}while(Date.now()<end);throw new Error(`timed out: ${what}`);};
  const el=(role,name)=>page.locator(`#a11y [role="${role}"][aria-label="${name}"]`).first();
  const text=start=>page.evaluate(s=>[...document.querySelectorAll('#a11y [data-key]')].map(e=>e.textContent).find(t=>t.startsWith(s)) ?? null,start);
  // the mirror's node of an element's name
  const node=name=>page.evaluate(n=>{for(const w of window.jfguiA11y.mirror.wins.values()) for(const x of w.nodes.values()) if(x.name===n) return x; return null;},name);

  const cdp=await page.context().newCDPSession(page);
  const axTree=async()=>{
    const {nodes}=await cdp.send('Accessibility.getFullAXTree');
    return nodes.filter(n=>!n.ignored).map(n=>({role:n.role?.value,name:(n.name?.value ?? '').trim(),description:n.description?.value ?? '',
      props:Object.fromEntries((n.properties || []).map(p=>[p.name,p.value?.value])),value:n.value?.value}));
  };
  const find=(tree,role,name)=>tree.find(n=>n.role===role && (name===undefined || n.name===name));
  // (a cell of a grid is a gridcell for Chromium)
  const ax=await axTree();
  if(process.env.A11Y_DUMP) console.log(JSON.stringify(ax.filter(n=>!['generic','none','StaticText','InlineTextBox'].includes(n.role)),null,0));
  // roles of their own
  for(const [role,name] of [['grid','Preise'],['row','Apfel 1,20 €'],['columnheader','Preis'],['rowheader','Apfel'],['gridcell','1,20 €'],
    ['banner','Kopf'],['navigation','Hauptmenü'],['complementary','Randspalte'],['main',undefined],['form','Bestellung'],['contentinfo','Fußzeile'],
    ['tree','Ordner'],['treeitem','Dokumente'],['radiogroup','Versand'],['toolbar','Werkzeuge'],['tooltip','Pflichtfeld: Vor- und Nachname'],
    ['log','Verlauf'],['meter','Speicher'],['spinbutton','Menge'],['scrollbar','Seite'],['article','Neuigkeiten'],
    ['figure','Umsatz 2026'],['separator',undefined]])
    ok(find(ax,role,name),`Chromium sees ${role} ${name ?? ''}`);
  // the further fields
  const docs=find(ax,'treeitem','Dokumente');
  ok(Number(docs.props.level)===1 && docs.props.expanded===false,'a tree item: level 1, collapsed');
  const name=find(ax,'textbox','Name');
  ok(name && name.props.required===true && name.props.invalid==='true' && name.description==='Vor- und Nachname','a text box: required, invalid, its description');
  ok(find(ax,'grid','Preise').props.multiselectable===true,'the table with several selected rows is a multi-selectable grid');
  // (a status line's name comes from its author only: what it says is its text)
  ok(find(ax,'status')?.props.live==='polite' && await page.locator('#a11y [role=status][aria-live=polite][data-key]').filter({hasText:'Bereit'}).count()===1,
    'the status line is a polite live region with its text');
  const meter=find(ax,'meter','Speicher');
  ok(Math.abs(Number(meter.props.valuenow ?? meter.value)-0.4)<1e-6 && await page.locator('#a11y [role=meter]').getAttribute('aria-valuetext')==='40 %',
    'the meter: its value and value text');
  ok(await page.locator('#a11y [role=cell][aria-rowindex="3"][aria-colindex="2"]').count()===1,'a cell: aria-rowindex / colindex');
  ok(await page.locator('#a11y [role=row][aria-label="Birne 0,90 €"][aria-posinset="2"][aria-setsize="2"]').count()===1,'a row: its place in the set');
  ok(await page.locator('#a11y [role=grid][aria-rowcount="3"][aria-colcount="2"]').count()===1,'the table: aria-rowcount / colcount');
  ok(find(ax,'heading','Neuigkeiten') && find(ax,'link','Impressum'),'out of view: a heading and a link of the article and the footer');
  // structure: the cell inside its row, the row inside its table
  ok(await page.locator('#a11y [role=grid][aria-label="Preise"] [role=row][aria-label="Apfel 1,20 €"] [role=cell]').count()===1,'the cell lies in its row in its table');

  // requests through the DOM
  await el('treeitem','Dokumente').dispatchEvent('click');
  await until(async()=>await page.locator('#a11y [role=treeitem][aria-label="Briefe"][aria-level="2"]').count()===1,'a click opens the tree item');
  checks++;
  ok(await el('treeitem','Dokumente').getAttribute('aria-expanded')==='true','the item: aria-expanded true');
  await page.evaluate(()=>{const r=document.querySelector('#a11y input[type=number]');r.value='4';r.dispatchEvent(new Event('input',{bubbles:true}));});
  await until(async()=>(await text('Name:'))?.includes('Menge: 4'),'the spin button set to 4');
  checks++;
  await el('link','Hilfe').focus();
  await until(async()=>await page.evaluate(()=>document.activeElement?.getAttribute('aria-label'))==='Hilfe','the DOM focus on the link');
  await until(async()=>(await node('Hilfe'))?.state&1,'the app focus on the link');
  checks+=2;
  // a focus on a link out of view: the page scrolls it into view first
  ok((await node('Impressum')).state&(1<<19),'the footer link is out of view');
  await el('link','Impressum').focus();
  await until(async()=>!((await node('Impressum'))?.state&(1<<19)),'the footer link scrolled into view');
  checks++;
  // an announcement: the shadow tree's polite live region says it
  await page.locator('#a11y [role=button][aria-label="Ansage"]').first().dispatchEvent('click');
  await until(async()=>await page.locator('#a11y [data-announce=polite]').textContent()==='Gespeichert','the announcement in the live region');
  checks++;
  await page.locator('#a11y [role=button][aria-label="Status"]').first().dispatchEvent('click');
  await until(async()=>await page.locator('#a11y [role=status][aria-live=polite][data-key]').filter({hasText:'Gespeichert'}).count()===1,'the status line changed');
  checks++;
  await el('button','Absenden').dispatchEvent('click');
  await until(async()=>await page.locator('#a11y [role=alert][aria-live=assertive]').filter({hasText:'Bitte einen Namen eingeben'}).count()===1,'an alert appears');
  checks++;
  ok(errors.length===0,'no page errors: '+errors.join('; '));
  console.log(`a11y web: ${checks} checks passed (Chromium accessibility tree of the rich format, requests, announcements)`);
} finally {
  await browser.close();
  server.close();
}
