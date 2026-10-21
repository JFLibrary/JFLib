// Fixed row sizes in the real shared WASM canvas, at three DPI/theme settings.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-exact-rows'), folder=resolve(build,'web/jfWord'), output=resolve(build,'exact-rows-browser');
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
  const html=await readFile(resolve('apps/jfWord/fixtures/exact-row-heights.html'));
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
        const label=async label=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label));await click(s.widgets.find(w=>w.label===label).id);};
        const number=async value=>{await click(3906);await page.keyboard.press('Control+a');await page.keyboard.insertText(String(value));await page.keyboard.press('Tab');};
        const rule=async value=>{await click(69441);await label(value);};
        const painted=async()=>{await page.waitForTimeout(120);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));};
        const exportDoc=async(format='docx')=>resource(page,(await action(page,'document.export',{format})).resources[0].uri);
        const inspect=async table=>(await action(page,'document.table.inspect',{table,limit:32})).value;
        const layout=async table=>(await action(page,'document.table.layout',{table,limit:8})).value;
        const select=async paragraph=>action(page,'document.select',{paragraph,start:0,end:0});
        await open();const backend=await page.evaluate(()=>window.__jfguiCompute);
        assert.equal(backend.mode,compute==='scalar'?'scalar':'simd');
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'exact-row-heights.html',mimeType:'text/html',buffer:html});
        await until(()=>state(page),s=>s.totalTables===1&&s.page.height===8640);
        let d=await state(page),table=d.tables[0].id,r=(await inspect(table)).rows[0],cell=r.cells[0].id,paragraph=r.cells[0].paragraphs[0];
        const height=async()=> (await layout(table)).pages.flatMap(p=>p.cells).filter(c=>c.id===cell&&!c.repeated).reduce((n,c)=>n+c.bounds[3],0);
        assert.equal(r.heightRule,'exact');assert.equal(await height(),480);
        const original=await exportDoc();await select(paragraph);await click(3918);await rule('Mindestens');await label('Übernehmen');
        await until(()=>inspect(table),r=>r.rows[0].heightRule==='atLeast');assert.ok(await height()>480);
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),original);await action(page,'document.redo');
        await click(3918);await rule('Genau');await number(0);const revision=(await state(page)).revision;await label('Übernehmen');
        await widget(3906);assert.equal((await state(page)).revision,revision); // Invalid height retains the modal transaction.
        await number(18);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await label('Übernehmen');await until(()=>inspect(table),r=>r.rows[0].heightRule==='exact'&&r.rows[0].height===360);assert.equal(await height(),360);
        // Direct input on the clipped first line, followed by revealing all stored text.
        const g=await layout(table),target=g.pages[0].cells.find(c=>c.id===cell);await select(paragraph);
        const sheet=await widget(0x300000);await page.mouse.click(...await point(sheet.rect[0]+(target.bounds[0]+220)*sheet.rect[2]/d.page.width,sheet.rect[1]+(target.bounds[1]+180)*sheet.rect[2]/d.page.width));
        await page.keyboard.press('Home');await page.keyboard.insertText('Web ');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===paragraph)?.text.includes('Web'));
        assert.equal(await height(),360);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const content=(await state(page)).paragraphs.find(p=>p.id===paragraph).text;
        await action(page,'document.table.change',{table,operation:'rowHeight',row:0,height:360,heightRule:'atLeast'});assert.ok(await height()>360);
        assert.equal((await state(page)).paragraphs.find(p=>p.id===paragraph).text,content);
        await action(page,'document.undo');assert.equal(await height(),360);
        // A row smaller than its padding is still selectable for recovery through properties.
        await action(page,'document.table.change',{table,operation:'rowHeight',row:0,height:80});
        const tiny=(await layout(table)).pages[0].cells.find(c=>c.id===cell);
        await select((await inspect(table)).rows[1].cells[0].paragraphs[0]);await painted();const tinySheet=await widget(0x300000);
        await page.mouse.click(...await point(tinySheet.rect[0]+(tiny.bounds[0]+300)*tinySheet.rect[2]/d.page.width,tinySheet.rect[1]+(tiny.bounds[1]+40)*tinySheet.rect[2]/d.page.width));
        await until(()=>state(page,'ui'),s=>s.widgets.some(w=>(w.label||'').includes('Tabelle · Zeile 1 · Spalte 1')));
        await click(3918);await number(18);await label('Übernehmen');assert.equal(await height(),360);
        const markup=(await exportDoc('html')).toString();assert.ok(markup.includes('data-jf-row-height="1 exact 360"')&&markup.includes('overflow:hidden'));
        assert.ok((await exportDoc('rtf')).toString().includes('\\trrh-360 '));
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'exact-rows.html',mimeType:'text/html',buffer:Buffer.from(markup)});
        await until(()=>state(page),s=>s.totalTables===1&&s.page.height===8640);table=(await state(page)).tables[0].id;
        assert.equal((await inspect(table)).rows[0].heightRule,'exact');
        const task=await action(page,'file.save',{path:'/documents/exact-rows.docx'});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path:'/documents/exact-rows.docx'});assert.deepEqual(await exportDoc(),bytes);
        assert.equal((await inspect((await state(page)).tables[0].id)).rows[0].heightRule,'exact');assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,backend,dialog:true,invalidHeight:true,directPageEdit:true,hiddenCellRecovery:true,htmlRoundtrip:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: exact row dialog, clipped input, hidden cell recovery, undo, HTML/RTF and durable DOCX`);
      }catch(error){const page=context.pages()[0];if(page){await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-failure.png`)}).catch(()=>{});await writeFile(resolve(output,`${engine}-${theme}-${scale}-failure.json`),JSON.stringify({error:String(error),ui:await state(page,'ui').catch(String),document:await state(page).catch(String)},null,2));}throw error;}finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
