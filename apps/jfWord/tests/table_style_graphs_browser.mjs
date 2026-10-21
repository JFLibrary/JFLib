// Inherited table templates and conditional areas in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-table-graphs'), folder=resolve(build,'web/jfWord'), output=resolve(build,'table-graphs-browser');
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
  const original=await readFile(resolve(build,'table-graph-tests/win-json.docx'));
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
        const click=async id=>{const w=await widget(id),c=await page.locator('#screen').boundingBox();await page.mouse.click(c.x+(w.rect[0]+w.rect[2]/2)/scale,c.y+(w.rect[1]+w.rect[3]/2)/scale);};
        const label=async name=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===name));await click(s.widgets.find(w=>w.label===name).id);};
        const choose=async(id,name)=>{await click(id);const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===name&&w.id!==id));await click(s.widgets.find(w=>w.label===name&&w.id!==id).id);};
        const input=async(id,text)=>{await click(id);await page.keyboard.press('Control+a');await page.keyboard.insertText(text);await page.keyboard.press('Tab');};
        const painted=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const exportDoc=async()=>resource(page,(await action(page,'document.export')).resources[0].uri);
        await open();
        await page.locator('#files').setInputFiles({name:'inherited-tables.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
        await until(()=>state(page),s=>s.path==='/imports/inherited-tables.docx');assert.deepEqual(await exportDoc(),original);
        const tid=(await state(page)).tables[0].id;
        const table=async()=>(await action(page,'document.table.inspect',{table:tid,limit:32})).value;
        const styles=async()=>(await action(page,'document.table.styles.list',{limit:32})).value.styles;
        const first=(await table()).rows[0].cells[0].paragraphs[0];
        await action(page,'document.select',{paragraph:first,start:0,end:0});
        await click(3928);await input(3940,'Browser · Vererbte Tabelle');
        await label('Vererbung');assert.equal((await widget(3949)).label,'ResearchChild');
        await input(3951,'3');await input(3952,'1');
        await label('Bereiche');await choose(3954,'Spaltenband 1');await input(3955,'DDEBDC');
        await choose(3954,'Oben links');await input(3955,'394E79');await label('Übernehmen');
        await until(table,t=>t.table.style==='jfTableCustom1'&&t.rows[0].cells[0].effectiveFill==='394E79');
        let definitions=await styles(),child=definitions.find(s=>s.id==='jfTableCustom1');
        assert.equal(child.basedOn,'ResearchChild');assert.equal(child.rowBandSize,3);assert.equal(child.columnBandSize,1);
        assert.equal((await table()).rows[1].cells[1].effectiveFill,'DDEBDC');
        // A parent edit arrives in the browser through the same action kernel.
        await action(page,'document.table.styles.put',{id:'ResearchBase',name:'ResearchBase',firstRow:{fill:'496A7D'}});
        assert.equal((await table()).rows[0].cells[1].effectiveFill,'496A7D');
        assert.equal((await table()).rows[0].cells[0].effectiveFill,'394E79');
        await click(3938);assert.equal((await table()).table.look.bandedColumns,false);
        await action(page,'document.undo');assert.equal((await table()).table.look.bandedColumns,true);
        // Reset just one corner to the live parent value, then undo and redo that edit.
        await click(3929);await label('Bereiche');await choose(3954,'Oben links');await click(3958);await label('Übernehmen');
        await until(table,t=>t.rows[0].cells[0].effectiveFill==='335577');
        await action(page,'document.undo');assert.equal((await table()).rows[0].cells[0].effectiveFill,'394E79');
        await action(page,'document.redo');assert.equal((await table()).rows[0].cells[0].effectiveFill,'335577');
        // Direct canvas input keeps all inherited formatting and stable paragraph identity.
        await action(page,'document.select',{paragraph:first,start:0,end:0});
        const layout=(await action(page,'document.table.layout',{table:tid,limit:8})).value;
        const cell=layout.pages[0].cells.find(c=>c.row===0&&c.column===0),sheet=await widget(0x300000),d=await state(page),canvas=await page.locator('#screen').boundingBox();
        await page.mouse.click(canvas.x+(sheet.rect[0]+(cell.bounds[0]+100)*sheet.rect[2]/d.page.width)/scale,canvas.y+(sheet.rect[1]+(cell.bounds[1]+180)*sheet.rect[2]/d.page.width)/scale);
        await page.keyboard.press('End');await page.keyboard.insertText(' · Web');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===first)?.text.includes('Web'));
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-table.png`)});
        const path='/documents/table-inheritance.docx',task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        await action(page,'document.select',{paragraph:first,start:0,end:0});
        await page.setViewportSize({width:680,height:800});
        await until(()=>state(page,'ui'),s=>s.widgets[0].rect[2]===680*scale);
        const bandControl=await widget(3938);assert.ok(bandControl.rect[0]+bandControl.rect[2]<=680*scale,'Vertical banding stays reachable at compact width');
        await click(3929);await label('Vererbung');
        await widget(3949);await painted();
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-inheritance.png`)});
        await label('Bereiche');await choose(3954,'Unten rechts');
        await until(()=>widget(3954),w=>w.label==='Unten rechts');await painted();
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-regions.png`)});
        assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,liveInheritance:true,guiRegions:true,bandSizes:true,cornerResetUndo:true,typedCell:true,durableReload:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: inherited templates, grouped/vertical bands, corners, reset/undo, canvas input and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
