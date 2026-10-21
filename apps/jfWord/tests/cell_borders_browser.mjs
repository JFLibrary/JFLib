// Per-cell borders and full-table clipboard in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-cell-borders'), folder=resolve(build,'web/jfWord'), output=resolve(build,'cell-borders-browser');
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
  const original=await readFile(resolve(build,'cell-border-tests/win-json.docx'));
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
        const click=async id=>{const w=await widget(id),c=await page.locator('#screen').boundingBox();await page.mouse.click(c.x+(w.rect[0]+w.rect[2]/2)/scale,c.y+(w.rect[1]+w.rect[3]/2)/scale);};
        const label=async name=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===name));await click(s.widgets.find(w=>w.label===name).id);};
        const choose=async(id,name)=>{await click(id);const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===name&&w.id!==id));await click(s.widgets.find(w=>w.label===name&&w.id!==id).id);};
        const input=async(id,text)=>{await click(id);await page.keyboard.press('Control+a');await page.keyboard.insertText(text);await page.keyboard.press('Tab');};
        const painted=()=>page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
        const exportDoc=async()=>resource(page,(await action(page,'document.export')).resources[0].uri);
        await open();
        await page.locator('#files').setInputFiles({name:'cell-borders.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
        await until(()=>state(page),s=>s.path==='/imports/cell-borders.docx');assert.deepEqual(await exportDoc(),original);
        let tid=(await state(page)).tables[0].id;
        const table=async()=>(await action(page,'document.table.inspect',{table:tid,limit:32})).value;
        let first=(await table()).rows[0].cells[0].paragraphs[0];
        await action(page,'document.select',{paragraph:first,start:0,end:0});
        await click(3965);await choose(3971,'Doppelt');await input(3972,'1.5');await input(3973,'264B78');
        await choose(3975,'Ausblenden');await choose(3979,'Gestrichelt');await input(3980,'1.25');await input(3981,'9C4B48');
        await choose(3983,'Gepunktet');await input(3984,'1');await input(3985,'665C8E');
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-edges.png`)});
        await label('Übernehmen');
        const edges={top:{style:'double',width:12,color:0x264B78},right:{style:'nil',width:0,color:0},bottom:{style:'dashed',width:10,color:0x9C4B48},left:{style:'dotted',width:8,color:0x665C8E}};
        await until(table,t=>t.rows[0].cells[0].borders.right?.style==='nil');assert.deepEqual((await table()).rows[0].cells[0].borders,edges);
        const revision=(await state(page)).revision;
        await click(3965);await input(3973,'GGGGGG');await label('Übernehmen');await widget(3973);
        assert.equal((await state(page)).revision,revision);await input(3973,'264B78');await label('Übernehmen');
        // Actual toolbar copy, body focus and browser Ctrl+V reach the same core as JSON/MCP.
        await click(3966);await click(3968);await page.keyboard.press('Control+v');await until(()=>state(page),s=>s.tables.length===2);
        await page.keyboard.press('Control+z');await until(()=>state(page),s=>s.tables.length===1);
        await page.keyboard.press('Control+y');await until(()=>state(page),s=>s.tables.length===2);
        await action(page,'clipboard.table.copy',{table:tid});await action(page,'document.new',{discardChanges:true});
        await action(page,'document.table.styles.put',{id:'StudyBase',name:'Studie',body:{fill:'FFCCCC'}});
        await action(page,'document.text.replace',{paragraph:1,start:0,end:0,text:'Browser · Tabelle'});
        await action(page,'document.select',{paragraph:1,start:0,end:0});await page.keyboard.press('Control+v');await until(()=>state(page),s=>s.tables.length===1);
        tid=(await state(page)).tables[0].id;first=(await table()).rows[0].cells[0].paragraphs[0];
        assert.deepEqual((await table()).rows[0].cells[0].borders,edges);
        const styles=(await action(page,'document.table.styles.list',{limit:32})).value.styles;
        const pastedStyle=(await table()).table.style;
        assert.notEqual(styles.find(s=>s.id===pastedStyle)?.basedOn,'StudyBase');
        // Typing directly on the page still works in a neighbouring cell.
        const textCell=(await table()).rows[0].cells[2].paragraphs[0];
        await action(page,'document.select',{paragraph:textCell,start:0,end:0});await page.keyboard.press('End');await page.keyboard.insertText(' · Web');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===textCell)?.text.includes('Web'));
        await action(page,'document.select',{paragraph:first,start:0,end:0});await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-table.png`)});
        const beforeCut=await exportDoc();await click(3967);await until(()=>state(page),s=>s.tables.length===0);
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),beforeCut);
        for(let row=0;row<5;row++)await action(page,'document.table.change',{table:tid,operation:'rowHeight',row,height:3000});
        const layout=(await action(page,'document.table.layout',{table:tid,limit:8})).value;
        assert.ok(layout.pageCount>=2);const repeated=layout.pages[1].cells.find(c=>c.row===0&&c.column===0&&c.repeated);
        assert.ok(repeated.edges.some(e=>e.side==='top'&&e.border.style==='double'));
        const path='/documents/cell-borders.docx',task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        await action(page,'document.select',{paragraph:first,start:0,end:0});await page.setViewportSize({width:680,height:800});
        await until(()=>state(page,'ui'),s=>s.widgets[0].rect[2]===680*scale);await click(3965);await widget(3985);await painted();
        const last=await widget(3985);assert.ok(last.rect[0]+last.rect[2]<=680*scale);
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-compact.png`)});await label('Abbrechen');
        assert.deepEqual(errors,[]);results.push({engine,theme,scale,compute,perEdgeGui:true,clipboardKeys:true,crossDocument:true,undoCut:true,repeatedHeaders:true,durableReload:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: edge dialog, merged cell clipboard, cross-document styles/sources, undo, page headers and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
