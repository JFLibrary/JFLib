// Live formula fields in the shared WASM canvas: actual dialog input, dependencies and storage.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-table-formulas'), folder=resolve(build,'web/jfWord'), output=resolve(build,'formulas-browser');
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
  for(const engine of (process.env.TABLES_ENGINE?[process.env.TABLES_ENGINE]:['chromium','firefox'])) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {for(const [scale,theme] of (process.env.TABLES_QUICK?[[1,'light']]:[[1,'light'],[1.5,'dark'],[2,'light']])) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(),errors=[];
        page.on('pageerror',e=>errors.push(String(e)));page.on('console',m=>{if(m.text().includes('UIWARN'))errors.push(m.text());});
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const compute=scale===1?'scalar':'auto';
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,null,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const label=async label=>{const s=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label));await click(s.widgets.find(w=>w.label===label).id);};
        const painted=async()=>{await page.waitForTimeout(120);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));};
        const exportDoc=async(format='docx')=>resource(page,(await action(page,'document.export',{format})).resources[0].uri);
        const inspect=async table=>(await action(page,'document.table.inspect',{table,limit:32})).value;
        const layout=async table=>(await action(page,'document.table.layout',{table,limit:8})).value;
        const select=async paragraph=>action(page,'document.select',{paragraph,start:0,end:0});
        await open();const backend=await page.evaluate(()=>window.__jfguiCompute);
        assert.equal(backend.mode,compute==='scalar'?'scalar':'simd');
        await action(page,'document.new',{discardChanges:true});
        await action(page,'document.page.set',{width:8640,height:10080,top:600,bottom:600,left:600,right:600});
        const table=(await action(page,'document.table.insert',{paragraph:1,offset:0,rows:12,columns:3})).value.id;
        const rows=(await inspect(table)).rows, cell=(r,c)=>rows[r].cells[c].paragraphs[0];
        const fields=async()=>(await action(page,'formulas.list',{limit:100})).value.fields;
        const result=async p=>(await fields()).find(f=>f.paragraph===p)?.result;
        const replace=async(p,text)=>{const old=(await state(page)).paragraphs.find(x=>x.id===p).text;await action(page,'document.text.replace',{paragraph:p,start:0,end:Buffer.byteLength(old),text});};
        const a2=cell(1,0),b2=cell(1,1),c2=cell(1,2),a3=cell(2,0),b3=cell(2,1),c3=cell(2,2),total=cell(11,2);
        for(const [p,text] of [[cell(0,0),'999'],[a2,'2'],[b2,'12,5'],[a3,'3'],[b3,'7.5']])await replace(p,text);
        await action(page,'document.table.change',{table,operation:'headerRows',headerRows:1});
        await select(c2);await action(page,'ui.command',{id:130});
        await widget(4110);await click(4110);await page.keyboard.press('Control+A');await page.keyboard.insertText('=A2*B2');
        await click(4111);await page.keyboard.insertText('0.00');await page.keyboard.press('Tab');
        await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label==='Ergebnis: 25.00'));
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await label('Übernehmen');await until(()=>fields(),f=>f.some(x=>x.paragraph===c2&&x.result==='25.00'));
        await action(page,'formulas.insert',{paragraph:c3,inline:0,offset:0,expression:'=R3C1*R3C2',numberFormat:'0.00'});
        await action(page,'formulas.insert',{paragraph:total,inline:0,offset:0,expression:'=SUM(ABOVE)',numberFormat:'0.00'});
        assert.equal(await result(total),'47.50');
        const before=await exportDoc();await replace(a2,'3');assert.equal(await result(total),'60.00');
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),before);
        await action(page,'document.redo');assert.equal(await result(c2),'37.50');await action(page,'document.undo');
        // Actual page keyboard edits a dependency; the generated total remains protected.
        await select(a2);await page.keyboard.press('Control+A');await page.keyboard.insertText('4');
        await until(()=>result(total),v=>v==='72.50');await action(page,'document.undo');assert.equal(await result(total),'47.50');
        const preview=await action(page,'formulas.evaluate',{paragraph:total,expression:'=IF(1,7,1/0)'});
        assert.deepEqual(preview.value,{result:'7',error:null});assert.deepEqual(await exportDoc(),before);
        await action(page,'formulas.update',{paragraph:c2,inline:1,expression:'=1/0'});assert.equal(await result(c2),'#DIV/0!');
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),before);
        await action(page,'clipboard.table.copy',{table});await select(1);await action(page,'clipboard.paste');assert.equal((await fields()).length,6);
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),before);
        const inner=(await action(page,'document.table.insert',{paragraph:a2,offset:0,rows:2,columns:1})).value.id;
        const ir=(await inspect(inner)).rows,ip=ir[0].cells[0].paragraphs[0],iq=ir[1].cells[0].paragraphs[0];
        await replace(ip,'11');await action(page,'formulas.insert',{paragraph:iq,inline:0,offset:0,expression:'=SUM(ABOVE)'});
        assert.equal(await result(iq),'11');assert.equal(await result(total),'47.50');
        for(let row=1;row<12;row++)await action(page,'document.table.change',{table,operation:'rowHeight',row,height:760});
        await action(page,'document.table.change',{table,operation:'position',floating:true,distance:[0,0,0,0]});
        const g=await layout(table);assert.ok(g.pageCount>=2);assert.equal(g.pages.flatMap(pg=>pg.cells).filter(c=>c.rowEnd&&!c.repeated).length,36);
        assert.equal(await result(total),'47.50');await select(c2);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const task=await action(page,'file.save',{path:'/documents/formulas.docx'});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path:'/documents/formulas.docx'});assert.deepEqual(await exportDoc(),bytes);
        assert.equal((await fields()).length,4);assert.equal(await result(total),'47.50');assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,backend,dialogTyping:true,liveDependencies:true,history:true,nestedFields:true,clipboard:true,floatingPages:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: formula dialog typing, page dependency edits, history, nested clipboard, floating pages and durable DOCX`);
      }catch(error){const page=context.pages()[0];if(page){await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-failure.png`)}).catch(()=>{});await writeFile(resolve(output,`${engine}-${theme}-${scale}-failure.json`),JSON.stringify({error:String(error),ui:await state(page,'ui').catch(String),document:await state(page).catch(String)},null,2));}throw error;}finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
