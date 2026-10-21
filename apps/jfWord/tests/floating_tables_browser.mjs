// Floating tables and geometrically different RTF row grids in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-floating-tables'), folder=resolve(build,'web/jfWord'), output=resolve(build,'floating-tables-browser');
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
  const varying=await readFile(resolve('apps/jfWord/fixtures/varying-grid-tables.rtf'),'utf8');
  const ragged=await readFile(resolve('apps/jfWord/fixtures/ragged-grid-tables.rtf'),'utf8');
  for(const engine of (process.env.TABLES_ENGINE?[process.env.TABLES_ENGINE]:['chromium','firefox'])) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {for(const [scale,theme] of (process.env.TABLES_QUICK?[[1,'light']]:[[1,'light'],[1.5,'dark'],[2,'light']])) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(),errors=[];
        page.on('pageerror',e=>errors.push(String(e)));page.on('console',m=>{if(m.text().includes('UIWARN'))errors.push(m.text());});
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const compute=scale===1?'scalar':'auto';
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const label=async name=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===name));await click(s.widgets.find(w=>w.label===name).id);};
        const set=async(id,text)=>{await click(id);await page.keyboard.press('Control+a');await page.keyboard.insertText(text);await page.keyboard.press('Tab');};
        const painted=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const exportDoc=async()=>resource(page,(await action(page,'document.export')).resources[0].uri);
        const inspect=async table=>(await action(page,'document.table.inspect',{table,limit:32})).value;
        const layout=async table=>(await action(page,'document.table.layout',{table,limit:8})).value;
        const select=async(paragraph,start=0,end=start)=>action(page,'document.select',{paragraph,start,end});
        const pasteRtf=async text=>page.evaluate(text=>{
          const data=new DataTransfer();data.setData('text/plain','RTF-Tabelle');data.setData('text/rtf',text);
          const input=document.querySelector('#input');input.focus({preventScroll:true});
          const event=new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true});
          if(event.clipboardData!==data)for(const type of data.types)event.clipboardData.setData(type,data.getData(type));
          input.dispatchEvent(event);
        },text);
        await open();await action(page,'document.new',{discardChanges:true});await select(1);
        await pasteRtf(varying);await until(()=>state(page),s=>s.totalTables===1);
        let d=await state(page);assert.deepEqual(d.tables[0].columns,[1800,200,1600,1800]);
        let info=await inspect(d.tables[0].id);assert.equal(info.rows.flatMap(r=>r.cells).filter(c=>c.paragraphs.length).length,5);
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'ragged.rtf',mimeType:'application/rtf',buffer:Buffer.from(ragged)});
        await until(()=>state(page),s=>s.totalTables===1&&s.dirty&&s.path==='');d=await state(page);info=await inspect(d.tables[0].id);
        assert.equal(info.rows[1].gridBefore,1);assert.equal(info.rows[1].gridAfter,1);assert.ok(info.rows[1].cells[0].omitted);
        await action(page,'document.new',{discardChanges:true});
        await action(page,'document.text.replace',{paragraph:1,start:0,end:0,text:'Text neben der Tabelle und darunter wieder über die volle Seitenbreite. '.repeat(70)});
        const table=(await action(page,'document.table.insert',{paragraph:1,offset:0,rows:3,columns:2})).value.id;
        const change=(operation,data={})=>action(page,'document.table.change',{table,operation,...data});
        await change('columns',{widths:[1440,1440]});await change('rowHeight',{row:0,height:660});await change('style',{style:'jfTableBlue'});
        const p=(await inspect(table)).rows[0].cells[0].paragraphs[0];
        await action(page,'document.text.replace',{paragraph:p,start:0,end:0,text:'Messung'});await select(p);
        const inline=await exportDoc();
        // The shared position dialog changes the same model as JSON/MCP.
        await click(3990);await click(4000);await page.keyboard.press('Home');await page.keyboard.press('ArrowDown');await page.keyboard.press('Enter');
        await widget(4002);await set(4002,'0');await set(4004,'0');await label('Übernehmen');
        await until(()=>state(page),s=>s.tables[0].position!==null);
        let g=await layout(table),first=g.pages[0],b=first.cells[0].bounds;
        assert.ok(first.bodyLines.find(l=>l.paragraph===1).bounds[0]>=b[0]+2880+120);
        assert.ok(first.bodyLines.some(l=>l.paragraph===1&&l.bounds[0]===b[0]));
        assert.ok(g.pageCount>1&&!g.pages[1].cells.length);
        const floating=await exportDoc();await action(page,'document.undo');assert.deepEqual(await exportDoc(),inline);await action(page,'document.redo');assert.deepEqual(await exportDoc(),floating);
        await select(p);await painted();
        const doc=await state(page),sheet=await widget(0x300000);
        const pagePoint=async(x,y)=>point(sheet.rect[0]+x*sheet.rect[2]/doc.page.width,sheet.rect[1]+y*sheet.rect[2]/doc.page.width);
        // Drag the move handle, then cancel a second gesture with Escape.
        const start=await pagePoint(b[0]-130,b[1]-130),end=await pagePoint(b[0]-130+360,b[1]-130+270);
        await page.mouse.move(...start);await page.mouse.down();await page.mouse.move(...end,{steps:6});await page.mouse.up();
        await until(()=>state(page),s=>s.tables[0].position.x===360&&s.tables[0].position.y===270);
        const moved=await exportDoc();g=await layout(table);b=g.pages[0].cells[0].bounds;
        const cancel=await pagePoint(b[0]-130,b[1]-130);await page.mouse.move(...cancel);await page.mouse.down();await page.mouse.move(cancel[0]+25,cancel[1]+20,{steps:4});await page.keyboard.press('Escape');await page.mouse.up();assert.deepEqual(await exportDoc(),moved);
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),floating);await action(page,'document.redo');assert.deepEqual(await exportDoc(),moved);
        // Edit directly inside the floating table's first cell.
        g=await layout(table);b=g.pages[0].cells[0].bounds;
        await page.mouse.click(...await pagePoint(b[0]+300,b[1]+250));await page.keyboard.press('End');await page.keyboard.insertText(' · Browser');
        await until(()=>state(page),s=>s.paragraphs.find(v=>v.id===p)?.text.includes('Browser'));
        // A rich paste is one transaction and can now grow a floating row over pages.
        const typed=await exportDoc(),typedRevision=(await state(page)).revision;
        await page.keyboard.press('End');await pasteRtf('{\\rtf1\\ansi '+('Overflow '.repeat(1800))+'}');await until(()=>state(page),s=>s.revision>typedRevision);
        assert.ok((await layout(table)).pageCount>2);await action(page,'document.undo');assert.deepEqual(await exportDoc(),typed);
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),moved);await action(page,'document.redo');assert.deepEqual(await exportDoc(),typed);
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-floating.png`)});
        const before=await exportDoc(),revision=(await state(page)).revision;
        const rejected=await page.evaluate(r=>window.jfguiAgent.request(r),{jsonrpc:'2.0',id:++serial,method:'actions/execute',params:{action:'document.table.change',input:{table,operation:'position',floating:true,y:31680},expectedRevision:revision}});
        assert.ok(rejected.error);assert.equal((await state(page)).revision,revision);assert.deepEqual(await exportDoc(),before);
        await click(3966);await select(1);await action(page,'clipboard.paste');assert.equal((await state(page)).totalTables,2);await action(page,'document.undo');assert.deepEqual(await exportDoc(),before);
        // Native/web transfers include both floating placement and omitted row intervals.
        await select(1);await pasteRtf(ragged);await until(()=>state(page),s=>s.totalTables===2);await select(1);await pasteRtf(varying);await until(()=>state(page),s=>s.totalTables===3);
        const path='/documents/floating-tables.docx',task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        await select(p);await page.setViewportSize({width:680,height:800});
        // The worker applies resize asynchronously. Do not click a stale wide-window rectangle.
        await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===4294931456&&w.rect[2]===680*scale));
        await painted();await click(3990);await widget(4009);await page.waitForTimeout(120);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-position.png`)});await label('Abbrechen');
        assert.deepEqual(errors,[]);results.push({engine,theme,scale,compute,varyingRtfClipboard:true,raggedRtfFile:true,positionDialog:true,moveHandle:true,escapeCancel:true,directCellEdit:true,atomicLayoutRejection:true,multiPageRichPaste:true,clipboard:true,durableReload:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: floating wrap, position dialog, move/cancel, cell typing, RTF grids, clipboard and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
