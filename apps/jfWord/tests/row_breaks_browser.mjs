// Row break policies and continued table editing in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-row-breaks'), folder=resolve(build,'web/jfWord'), output=resolve(build,'row-breaks-browser');
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
  const html=await readFile(resolve('apps/jfWord/fixtures/row-breaks.html'));
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
        const accept=async()=>{const w=(await state(page,'ui')).widgets.find(w=>w.label==='Übernehmen');assert.ok(w);await click(w.id);};
        const painted=async()=>{await page.waitForTimeout(120);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));};
        const exportDoc=async(format='docx')=>resource(page,(await action(page,'document.export',{format})).resources[0].uri);
        const inspect=async table=>(await action(page,'document.table.inspect',{table,limit:32})).value;
        const layout=async table=>(await action(page,'document.table.layout',{table,limit:8})).value;
        const select=async(paragraph)=>action(page,'document.select',{paragraph,start:0,end:0});
        await open();await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'row-breaks.html',mimeType:'text/html',buffer:html});
        await until(()=>state(page),s=>s.totalTables===1&&s.page.height===5760);
        let d=await state(page),table=d.tables[0].id,r=(await inspect(table)).rows[0],cell=r.cells[0].id,paragraph=r.cells[0].paragraphs[0];
        assert.ok(r.allowSplit);assert.ok((await layout(table)).pages[0].cells.some(c=>c.id===cell&&!c.rowEnd));
        const original=await exportDoc();await select(paragraph);await click(3918);await click(3909);
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await accept();await until(()=>inspect(table),r=>!r.rows[0].allowSplit);
        let g=await layout(table);assert.ok(!g.pages[0].cells.some(c=>c.id===cell));assert.equal(g.pages.flatMap(p=>p.cells).filter(c=>c.id===cell).length,1);
        const kept=await exportDoc();await action(page,'document.undo');assert.deepEqual(await exportDoc(),original);await action(page,'document.redo');assert.deepEqual(await exportDoc(),kept);
        // Real canvas input into the row after its move to the following page.
        const target=g.pages[1].cells.find(c=>c.id===cell);assert.ok(target);await select(paragraph);
        const at=async()=>{const sheet=await widget(0x300001);return point(sheet.rect[0]+(target.bounds[0]+250)*sheet.rect[2]/d.page.width,sheet.rect[1]+(target.bounds[1]+250)*sheet.rect[2]/d.page.width);};
        for(let i=0;i<25;i++){const p=await at();if(p[1]>280&&p[1]<890)break;await page.mouse.move(1120,650);await page.mouse.wheel(0,p[1]>890?350:-350);await painted();}
        const pos=await at();assert.ok(pos[1]>250&&pos[1]<950,pos);await page.mouse.click(...pos);await page.keyboard.press('End');await page.keyboard.insertText(' · Web');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===paragraph)?.text.includes('Web'));
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const markup=(await exportDoc('html')).toString();assert.ok(markup.includes('break-inside:avoid'));assert.ok((await exportDoc('rtf')).toString().includes('\\trkeep '));
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'row-policy.html',mimeType:'text/html',buffer:Buffer.from(markup)});
        await until(()=>state(page),s=>s.totalTables===1&&s.page.height===5760);table=(await state(page)).tables[0].id;
        assert.ok(!(await inspect(table)).rows[0].allowSplit);
        const task=await action(page,'file.save',{path:'/documents/row-policy.docx'});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path:'/documents/row-policy.docx'});assert.deepEqual(await exportDoc(),bytes);
        assert.ok(!(await inspect((await state(page)).tables[0].id)).rows[0].allowSplit);assert.deepEqual(errors,[]);
        const conflict=await readFile(resolve('apps/jfWord/fixtures/interleaved-row-breaks.html'));
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'interleaved.html',mimeType:'text/html',buffer:conflict});
        await until(()=>state(page),s=>s.totalTables===3);
        assert.ok((await layout((await state(page)).tables[0].id)).pageCount>1);
        assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,dialog:true,directPageEdit:true,htmlRoundtrip:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: row keep dialog, continuation input, undo, HTML/RTF and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
