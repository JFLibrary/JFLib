// Actual WASM mouse/keyboard editing of merged cells, sharing the native document model.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-merged'), folder=resolve(build,'web/jfWord'), output=resolve(build,'table-merges-browser');
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
  const original=await readFile(resolve(build,'table-tests/win-json.docx'));
  const mergedNative=await readFile(resolve(build,'table-merge-tests/win-json.docx'));
  for(const engine of (process.env.TABLES_ENGINE?[process.env.TABLES_ENGINE]:['chromium','firefox'])) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try { for(const [scale,theme] of (process.env.TABLES_QUICK?[[1,'light']]:[[1,'light'],[1.5,'dark'],[2,'light']])) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(), errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const compute=scale===1?'scalar':'auto';
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const choose=async(id,label)=>{await click(id);const menu=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label && w.id!==id));await click(menu.widgets.find(w=>w.label===label && w.id!==id).id);};
        const confirm=async()=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label==='Übernehmen'));await click(s.widgets.find(w=>w.label==='Übernehmen').id);};
        const input=async(id,text)=>{await click(id);await page.keyboard.press('Control+a');await page.keyboard.insertText(text);await page.keyboard.press('Tab');};
        const physical=async(x,y)=>{const sheet=await widget(0x300000),d=await state(page);return point(sheet.rect[0]+x*sheet.rect[2]/d.page.width,sheet.rect[1]+y*sheet.rect[2]/d.page.width);};
        const drag=async(from,to)=>{await page.mouse.move(...from);await page.mouse.down();await page.mouse.move(...to,{steps:8});await page.mouse.up();};
        const importFile=async(buffer,name)=>{await page.locator('#files').setInputFiles({name,mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer});await until(()=>state(page),s=>s.path===`/imports/${name}`);};
        const exported=async()=>resource(page,(await action(page,'document.export')).resources[0].uri);
        await open();
        await importFile(mergedNative,'merged-native.docx');
        assert.deepEqual(await exported(),mergedNative);
        await importFile(original,'rectangular-native.docx');
        assert.deepEqual(await exported(),original);
        const tid=(await state(page)).tables[0].id;
        const table=async()=>(await action(page,'document.table.inspect',{table:tid})).value;
        const text=async id=>(await state(page)).paragraphs.find(p=>p.id===id)?.text;
        const select=async(paragraph,start=0,end=start)=>action(page,'document.select',{paragraph,start,end});
        const change=async(operation,data)=>action(page,'document.table.change',{table:tid,operation,...data});
        const before=await table(), first=before.rows[0].cells[0].paragraphs[0], third=before.rows[0].cells[2].paragraphs[0];
        await select(first);await click(3921);
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await confirm();
        await until(table,t=>t.rows[0].cells[0].rowSpan===2 && t.rows[0].cells[0].columnSpan===2);
        assert.equal((await table()).rows[0].cells[0].paragraphs.length,5);
        // Hit the covered lower/right area; text still edits the single owning cell.
        await change('rowHeight',{row:0,height:1440});await change('rowHeight',{row:1,height:1440});
        const d=await state(page);
        await page.mouse.click(...await physical(d.page.left+4200,d.page.top+2400));
        await page.keyboard.press('Control+Home');await page.keyboard.insertText('Verbunden: ');
        await until(()=>text(first),s=>s==='Verbunden: Artikel');
        await page.keyboard.press('Tab');await page.keyboard.press('End');await page.keyboard.insertText('!');
        await until(()=>text(third),s=>s==='Preis!');
        await page.keyboard.press('Shift+Tab');
        // A merged edge changes the last covered grid column/row, once per drag.
        let revision=(await state(page)).revision;
        await drag(await physical(d.page.left+5200,d.page.top+1000),await physical(d.page.left+5380,d.page.top+1000));
        await until(table,t=>t.table.columns[1]===2180 && t.table.columns[2]===1820);
        assert.equal((await state(page)).revision,revision+1);
        await action(page,'document.undo');await until(table,t=>t.table.columns[1]===2000);
        revision=(await state(page)).revision;
        await drag(await physical(d.page.left+1000,d.page.top+2880),await physical(d.page.left+1000,d.page.top+3060));
        await until(table,t=>t.rows[1].height===1620);
        assert.equal((await state(page)).revision,revision+1);
        await action(page,'document.undo');
        // Properties accept the whole merged width; split is exactly undoable.
        await select(first);await click(3918);await input(3905,'260.05');await confirm();
        await until(table,t=>t.table.columns[0]+t.table.columns[1]===5201);
        await action(page,'document.undo');await select(first);
        const joined=await table();await click(3922);
        await until(table,t=>!t.rows[1].cells[1].covered);
        assert.deepEqual((await table()).rows[0].cells[0].paragraphs,joined.rows[0].cells[0].paragraphs);
        await action(page,'document.undo');assert.deepEqual(await table(),joined);
        // GUI and semantic actions use identical font measurement and column fitting.
        for(const [mode,label] of [['contents','An Inhalt'],['page','An Seitenbreite'],['equal','Gleich verteilen']]) {
          const previous=await table(),rev=(await state(page)).revision;
          await action(page,'document.table.fit',{table:tid,mode});const expected=(await table()).table.columns;
          const changed=(await state(page)).revision!==rev;
          if(changed)await action(page,'document.undo');
          await select(first);await choose(3923,label);
          await until(table,t=>JSON.stringify(t.table.columns)===JSON.stringify(expected));
          if(changed)await action(page,'document.undo');
          assert.deepEqual(await table(),previous);
        }
        // Adjacent horizontal-only and vertical-only merges coexist in the same grid.
        const last=(await table()).rows[2].cells[0].paragraphs[0];
        await select(last);await click(3921);await confirm();
        await until(table,t=>t.rows[2].cells[0].columnSpan===2);
        await select(third);await click(3921);await input(3901,'3');await confirm();
        await until(table,t=>t.rows[0].cells[2].rowSpan===3);
        await select(first);
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const path='/documents/merged-table.docx', task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
        const bytes=await exported();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exported(),bytes);
        await select(first);await page.setViewportSize({width:1000,height:800});
        await widget(3923);
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-narrow.png`)});
        assert.deepEqual(errors,[]);
        results.push({engine,theme,scale,compute,bytes:bytes.length,nativeTransfer:true,mergeDialog:true,coveredHit:true,tabSkipsCovered:true,resizeSpans:true,properties:true,splitUndo:true,guiAgentFit:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: merged page editing, covered hit/Tab, resize, split, GUI/API fitting, DOCX and durable reload`);
      } finally {await context.close();}
    } } finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
