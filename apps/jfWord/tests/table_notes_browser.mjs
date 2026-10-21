// Cell notes in the shared WASM canvas, including real typing, navigation and storage.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-table-notes'), folder=resolve(build,'web/jfWord'), output=resolve(build,'table-notes-browser');
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
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}&compute=${compute}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
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
        await action(page,'document.page.set',{width:8640,height:8640,top:600,bottom:600,left:600,right:600});
        let table=(await action(page,'document.table.insert',{paragraph:1,offset:0,rows:12,columns:2})).value.id;
        let rows=(await inspect(table)).rows,p=rows[0].cells[0].paragraphs[0],q=rows[6].cells[0].paragraphs[0];
        await action(page,'document.text.replace',{paragraph:p,start:0,end:0,text:'Messwert α'});
        await select(p);await action(page,'ui.command',{id:23});
        let d=await until(()=>state(page),d=>d.totalNotes===1),first=d.notes[0];
        assert.equal(first.anchor.paragraph,p);assert.equal(first.number,1);
        await page.keyboard.insertText('Browser-Zellnote β');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===first.paragraphs[0])?.text==='Browser-Zellnote β');
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-note.png`)});
        await action(page,'ui.command',{id:25});
        await until(()=>state(page,'ui'),s=>s.widgets.some(w=>(w.label||'').includes('Tabelle · Zeile 1 · Spalte 1')));
        // The return command selects the native field without flattening its marker.
        await action(page,'notes.insert',{paragraph:q,inline:0,offset:0,kind:'endnote',text:'Zusätzlicher Nachweis Ω'});
        d=await state(page);let end=d.notes.find(n=>n.kind==='endnote');assert.equal(end.anchor.paragraph,q);
        const before=await exportDoc();await action(page,'notes.kind',{id:end.id,kind:'footnote'});
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),before);
        await action(page,'document.redo');assert.equal((await state(page)).notes.find(n=>n.id===end.id).kind,'footnote');
        await action(page,'document.undo');
        // Nested cell note, shared table clipboard and fresh note identities.
        const inner=(await action(page,'document.table.insert',{paragraph:p,offset:0,rows:1,columns:1})).value.id;
        const ip=(await inspect(inner)).rows[0].cells[0].paragraphs[0];
        await action(page,'notes.insert',{paragraph:ip,inline:0,offset:0,kind:'footnote',text:'Innere Zellnote'});
        d=await state(page);assert.equal(d.notes.find(n=>n.id===first.id).number,2);
        const copied=await exportDoc();await action(page,'clipboard.table.copy',{table});await select(1);await action(page,'clipboard.paste');
        assert.equal((await state(page)).totalNotes,6);await action(page,'document.undo');assert.deepEqual(await exportDoc(),copied);
        await action(page,'document.table.change',{table,operation:'headerRows',headerRows:1});
        for(let row=1;row<12;row++)await action(page,'document.table.change',{table,operation:'rowHeight',row,height:760});
        await action(page,'document.table.change',{table,operation:'position',floating:true,distance:[0,0,0,0]});
        const g=await layout(table);assert.ok(g.pageCount>=2);
        d=await state(page);for(const n of d.notes.filter(n=>n.kind==='footnote')){
          const hits=g.pages.flatMap(pg=>pg.bodyLines.filter(l=>n.paragraphs.includes(l.paragraph)).map(line=>({pg,line})));
          assert.ok(hits.length);for(const {pg,line} of hits)assert.ok(pg.cells.every(c=>c.bounds[1]+c.bounds[3]<line.bounds[1]));
        }
        assert.equal(g.pages.flatMap(pg=>pg.cells).filter(c=>c.rowEnd&&!c.repeated).length,24);
        await select(p);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const task=await action(page,'file.save',{path:'/documents/table-notes.docx'});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path:'/documents/table-notes.docx'});assert.deepEqual(await exportDoc(),bytes);
        assert.equal((await state(page)).totalNotes,3);assert.deepEqual(errors,[]);
        results.push({engine,scale,theme,compute,backend,directNoteTyping:true,anchorReturn:true,nestedNotes:true,clipboard:true,floatingPages:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: cell-note typing, anchor return, history, nested clipboard, floating pages and durable DOCX`);
      }catch(error){const page=context.pages()[0];if(page){await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-failure.png`)}).catch(()=>{});await writeFile(resolve(output,`${engine}-${theme}-${scale}-failure.json`),JSON.stringify({error:String(error),ui:await state(page,'ui').catch(String),document:await state(page).catch(String)},null,2));}throw error;}finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
