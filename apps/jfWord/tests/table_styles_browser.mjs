// Shared repeated-header and table-template flows in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-table-styles'), folder=resolve(build,'web/jfWord'), output=resolve(build,'table-styles-browser');
await mkdir(output,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const server=createServer(async(req,res)=>{
  try {
    const name=decodeURIComponent(new URL(req.url,'http://localhost').pathname).slice(1) || 'index.html';
    if(name==='favicon.ico'){res.writeHead(204).end();return;}
    const path=resolve(folder,name);if(!path.startsWith(folder+sep))throw Error('Invalid path');
    res.writeHead(200,{'content-type':{'.js':'text/javascript','.html':'text/html','.css':'text/css','.wasm':'application/wasm'}[extname(path)] || 'application/octet-stream','cache-control':'no-store'}).end(await readFile(path));
  } catch(e){res.writeHead(404).end(String(e));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`, results=[];
let serial=0;
async function rpc(page,method,params={}) {
  const reply=await page.evaluate(r=>window.jfguiAgent.request(r),{jsonrpc:'2.0',id:++serial,method,params});
  assert.ok(!reply.error,JSON.stringify(reply));return reply.result;
}
const state=(page,domain='document')=>rpc(page,'state/inspect',{domain,limit:domain==='ui'?500:100});
const action=async(page,action,input={})=>rpc(page,'actions/execute',{action,input,expectedRevision:(await state(page)).revision});
async function until(fn,ok) {let last;const end=Date.now()+20000;do {last=await fn();if(ok(last))return last;await new Promise(r=>setTimeout(r,80));}while(Date.now()<end);throw Error(JSON.stringify(last));}
async function resource(page,uri){let offset=0;const data=[];for(;;){const p=await rpc(page,'resources/read',{uri,offset,limit:65536});data.push(Buffer.from(p.data,'base64'));if(p.eof)return Buffer.concat(data);offset=p.nextOffset;}}
try {
  const original=await readFile(resolve(build,'table-style-tests/win-json.docx'));
  for(const engine of (process.env.TABLES_ENGINE?[process.env.TABLES_ENGINE]:['chromium','firefox'])) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {for(const [scale,theme] of (process.env.TABLES_QUICK?[[1,'light']]:[[1,'light'],[1.5,'dark'],[2,'light']])) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(), errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        page.on('console',m=>{if(m.text().includes('UIWARN'))errors.push(m.text());});
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const compute=scale===1?'scalar':'auto';
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const choose=async(id,label)=>{await click(id);const menu=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label && w.id!==id));await click(menu.widgets.find(w=>w.label===label && w.id!==id).id);};
        const confirm=async()=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label==='Übernehmen'));await click(s.widgets.find(w=>w.label==='Übernehmen').id);};
        const input=async(id,text)=>{await click(id);await page.keyboard.press('Control+a');await page.keyboard.insertText(text);await page.keyboard.press('Tab');};
        const exportDoc=async()=>resource(page,(await action(page,'document.export')).resources[0].uri);
        await open();
        await page.locator('#files').setInputFiles({name:'headers-native.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
        await until(()=>state(page),s=>s.path==='/imports/headers-native.docx');
        assert.deepEqual(await exportDoc(),original);
        const tid=(await state(page)).tables[0].id;
        const table=async()=>(await action(page,'document.table.inspect',{table:tid,limit:32})).value;
        const layout=async()=>(await action(page,'document.table.layout',{table:tid,limit:8})).value;
        const select=async(paragraph)=>action(page,'document.select',{paragraph,start:0,end:0});
        const first=(await table()).rows[0].cells[0].paragraphs[0];
        const secondHeader=(await layout()).pages[1].cells.find(c=>c.row===0 && c.column===0);
        assert.ok(secondHeader.repeated);
        await select(first);
        // Change a template through the native widgets, including creation and live edits.
        await choose(3926,'Blau · Zeilenbänder');
        await until(table,t=>t.table.style==='jfTableBlue');
        await click(3928);
        await input(3940,'Browser · Forschung');await input(3942,'345A68');await confirm();
        await until(table,t=>t.table.style==='jfTableCustom1' && t.rows[0].cells[0].effectiveFill==='345A68');
        await click(3929);await input(3943,'D5E9F3');await confirm();
        await until(table,t=>t.rows[2].cells[0].effectiveFill==='D5E9F3');
        await click(3937);await until(table,t=>!t.table.look.bandedRows);
        await action(page,'document.undo');assert.ok((await table()).table.look.bandedRows);
        // Repeated header count is a document transaction, not a per-page visual toggle.
        await input(3927,'0');await until(table,t=>t.table.headerRows===0);
        assert.ok((await layout()).pages.every(p=>p.cells.every(c=>!c.repeated)));
        await action(page,'document.undo');assert.equal((await table()).table.headerRows,2);
        // Scroll to the second page, hit the repeated merged cell, and type on that page.
        const canvas=await page.locator('#screen').boundingBox();
        await page.mouse.move(canvas.x+canvas.width*0.70,canvas.y+canvas.height*0.65);
        await page.mouse.wheel(0,560);
        await until(()=>widget(0x300001),w=>w.rect[1]/scale<800);
        const sheet=await widget(0x300001), d=await state(page);
        const [x,y]=secondHeader.bounds;
        await page.mouse.click(...await point(sheet.rect[0]+(x+300)*sheet.rect[2]/d.page.width,sheet.rect[1]+(y+180)*sheet.rect[2]/d.page.width));
        await page.keyboard.press('Control+Home');await page.keyboard.press('End');await page.keyboard.insertText(' [Web]');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===first)?.text==='Messreihe [Web]');
        const afterSheet=await widget(0x300001);
        assert.ok(afterSheet.rect[1]/scale<800,'Editing a repeated header must stay on the chosen page');
        const pageStatus=(await state(page,'ui')).widgets.find(w=>w.label?.startsWith('Seite 2 von'));
        assert.ok(pageStatus,'The page indicator follows the selected header instance');
        const actual=(await layout()).pages;
        assert.equal(actual.flatMap(p=>p.cells).filter(c=>!c.repeated).length,63);
        assert.ok(actual.filter(p=>p.cells.length).every(p=>p.cells.filter(c=>c.row<2).length===3));
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-repeated.png`)});
        const path='/documents/header-styles.docx', task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        await select(first);await page.setViewportSize({width:680,height:800});
        await click(3929);
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-compact.png`)});
        assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,nativeTransfer:true,guiTemplates:true,headerToggleUndo:true,editRepeatedHeader:true,mergedHeaders:true,durableReload:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: template dialog, banding, repeat toggle/undo, direct repeated-header editing, DOCX and durable reload`);
      } finally {await context.close();}
    }} finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
