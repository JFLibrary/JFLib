// Real WASM page editing, table geometry, clipboard, history and durable transfer.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-tables'), folder=resolve(build,'web/jfWord'), output=resolve(build,'tables-browser');
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
  const response=await page.evaluate(r=>window.jfguiAgent.request(r),{jsonrpc:'2.0',id:++serial,method,params});
  assert.ok(!response.error,JSON.stringify(response));return response.result;
}
const state=(page,domain='document')=>rpc(page,'state/inspect',{domain,limit:domain==='ui'?500:100});
const action=async(page,action,input={})=>rpc(page,'actions/execute',{action,input,expectedRevision:(await state(page)).revision});
async function until(fn,ok) {let last;const end=Date.now()+20000;do {last=await fn();if(ok(last))return last;await new Promise(r=>setTimeout(r,80));}while(Date.now()<end);throw Error(JSON.stringify(last));}
async function resource(page,uri){let offset=0;const data=[];for(;;){const p=await rpc(page,'resources/read',{uri,offset,limit:65536});data.push(Buffer.from(p.data,'base64'));if(p.eof)return Buffer.concat(data);offset=p.nextOffset;}}
try {
  const original=await readFile(resolve(build,'table-tests/win-json.docx'));
  for(const engine of (process.env.TABLES_ENGINE?[process.env.TABLES_ENGINE]:['chromium','firefox'])) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try { for(const [scale,theme] of (process.env.TABLES_QUICK?[[1,'light']]:[[1,'light'],[1.5,'dark'],[2,'light']])) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(), errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const compute=scale===1?"scalar":"auto";
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const choose=async(id,label)=>{await click(id);const menu=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label && w.id!==id));await click(menu.widgets.find(w=>w.label===label && w.id!==id).id);};
        const drag=async(from,to,cancel=false)=>{await page.mouse.move(...from);await page.mouse.down();await page.mouse.move(...to,{steps:8});if(cancel)await page.keyboard.press('Escape');await page.mouse.up();};
        const physical=async(x,y)=>{const sheet=await widget(0x300000),d=await state(page);return point(sheet.rect[0]+x*sheet.rect[2]/d.page.width,sheet.rect[1]+y*sheet.rect[2]/d.page.width);};
        const trace=async name=>{if(process.env.TABLES_TRACE){console.log(name,JSON.stringify(await state(page)));await page.screenshot({path:resolve(output,`trace-${name}.png`)});}};
        await open();
        await page.locator('#files').setInputFiles({name:'tables-native.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
        await until(()=>state(page),s=>s.path==='/imports/tables-native.docx');
        assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),original);
        const tid=(await state(page)).tables[0].id;
        const table=async()=>(await action(page,'document.table.inspect',{table:tid})).value;
        const text=async id=>(await state(page)).paragraphs.find(p=>p.id===id)?.text;
        const select=async(paragraph,start=0,end=start)=>action(page,'document.select',{paragraph,start,end});
        const first=(await table()).rows[0].cells[0].paragraphs[0];
        const second=(await table()).rows[0].cells[1].paragraphs[0];
        const third=(await table()).rows[0].cells[2].paragraphs[0];
        const d=await state(page);
        // One drag = one undo transaction, independent of DPI.
        await select(first);
        let revision=(await state(page)).revision;
        await drag(await physical(d.page.left+3200,d.page.top+300),await physical(d.page.left+3920,d.page.top+300));
        await until(table,t=>t.table.columns[0]===3920 && t.table.columns[1]===1280);
        assert.equal((await state(page)).revision,revision+1);
        await page.keyboard.press('Control+z');await until(table,t=>t.table.columns[0]===3200);
        revision=(await state(page)).revision;
        await drag(await physical(d.page.left+3200,d.page.top+300),await physical(d.page.left+3560,d.page.top+300),true);
        assert.equal((await state(page)).revision,revision);
        await drag(await physical(d.page.left+1000,d.page.top+720),await physical(d.page.left+1000,d.page.top+1080));
        await until(table,t=>t.rows[0].height===1080);
        await trace('resized');
        // A real page click activates the cell story, then editing cannot cross its boundaries.
        await page.mouse.click(...await physical(d.page.left+500,d.page.top+300));
        await widget(3918);
        await page.keyboard.press('Home');await page.keyboard.insertText('Bio ');
        await until(()=>text(first),s=>s==='Bio Artikel');
        await page.keyboard.press('Control+z');await until(()=>text(first),s=>s==='Artikel');
        await page.keyboard.press('Tab');await page.keyboard.press('End');await page.keyboard.insertText('!');
        await until(()=>text(second),s=>s==='Menge!');
        await page.keyboard.press('Shift+Tab');await page.keyboard.press('End');await page.keyboard.press('Enter');await page.keyboard.insertText('Kopf');
        await until(table,t=>t.rows[0].cells[0].paragraphs.length===2);
        assert.equal(await text(second),'Menge!');
        await select(first,0,7);await page.keyboard.press('Control+b');
        await select(third,0,5);await action(page,'clipboard.copy');
        await select(second,0,6);await action(page,'clipboard.paste');
        await until(()=>text(second),s=>s==='Preis');
        await trace('typing');
        // The horizontal ruler uses the selected cell's text origin, including padding.
        await select(second);
        const ruler=await widget(2852),sheet=await widget(0x300000);
        const cellOrigin=d.page.left+(await table()).table.columns[0]+90;
        await page.mouse.click(...await point(sheet.rect[0]+(cellOrigin+720)*sheet.rect[2]/d.page.width,ruler.rect[1]+12*scale));
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===second).effectiveParagraphFormat.tabs.some(t=>t.position===720));
        // Ribbon/context actions and the shared exact-measurement dialog.
        await click(3912);await until(table,t=>t.table.rowCount===4);
        await select((await table()).rows[1].cells[1].paragraphs[0]);
        await click(3915);await until(table,t=>t.table.rowCount===3);
        await select(second);await click(3914);await until(table,t=>t.table.columns.length===4);
        await select((await table()).rows[0].cells[2].paragraphs[0]);
        await click(3916);await until(table,t=>t.table.columns.length===3);
        const active=(await table()).rows[0].cells[0].paragraphs[0];
        await select(active);await click(3918);await click(3905);await page.keyboard.press('Control+a');await page.keyboard.insertText('150.05');await page.keyboard.press('Tab');
        await click(3906);await page.keyboard.press('Control+a');await page.keyboard.insertText('54');await page.keyboard.press('Tab');
        await choose(3907,'Hellgelb');await choose(3908,'Unten');
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await click((await state(page,'ui')).widgets.find(w=>w.label==='Übernehmen').id);
        await until(table,t=>t.table.columns[0]===3001 && t.rows[0].height===1080 && t.rows[0].cells[0].fill==='FFF2CC' && t.rows[0].cells[0].alignment==='bottom');
        // Tab from the last cell creates exactly one new row; Enter stays within a cell.
        let t=await table();const last=t.rows.at(-1).cells.at(-1).paragraphs[0];
        await select(last);await page.keyboard.press('Tab');await until(table,t=>t.table.rowCount===4);
        await page.keyboard.insertText('Neue Zeile');
        t=await table();assert.equal(await text(t.rows.at(-1).cells[0].paragraphs[0]),'Neue Zeile');
        await page.keyboard.press('Control+Tab');await page.keyboard.insertText('Wert');
        await until(()=>text(t.rows.at(-1).cells[0].paragraphs[0]),s=>s==='Neue Zeile\tWert');
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const path='/documents/tables-browser.docx';const task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
        const bytes=await resource(page,(await action(page,'document.export')).resources[0].uri);
        await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});
        assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),bytes);
        await action(page,'document.new',{discardChanges:true});
        await click(2003);await click(1514);
        await click(3901);await page.keyboard.press('Control+a');await page.keyboard.insertText('2');await page.keyboard.press('Tab');
        await click(3902);await page.keyboard.press('Control+a');await page.keyboard.insertText('2');await page.keyboard.press('Tab');
        await click((await state(page,'ui')).widgets.find(w=>w.label==='Übernehmen').id);
        await until(()=>state(page),s=>s.tables.length===1 && s.tables[0].rowCount===2 && s.tables[0].columns.length===2);
        await page.keyboard.insertText('Neue Tabelle');await page.keyboard.press('Tab');await page.keyboard.insertText('Zweite Zelle');
        await until(()=>state(page),s=>s.paragraphs.some(p=>p.text==='Neue Tabelle') && s.paragraphs.some(p=>p.text==='Zweite Zelle'));
        assert.deepEqual(errors,[]);
        results.push({engine,theme,scale,compute,bytes:bytes.length,cellEditing:true,insertDialog:true,cellRuler:true,tableGestures:true,undo:true,clipboard:true,preciseDialog:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: page cells, rows/columns, resize, clipboard, undo, properties and persistence`);
      } finally {await context.close();}
    } } finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
