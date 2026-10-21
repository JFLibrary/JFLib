// Multi-page floating tables and exact HTML positions in the real WASM UI.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-floating-pages'), folder=resolve(build,'web/jfWord'), output=resolve(build,'floating-pages-browser');
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
  const html=await readFile(resolve('apps/jfWord/fixtures/floating-pages.html'));
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
        const painted=async()=>{await page.waitForTimeout(120);await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));};
        const exportDoc=async(format='docx')=>{const r=(await action(page,'document.export',{format})).resources[0];if(format==='html')assert.equal(r.mimeType,'text/html');return resource(page,r.uri);};
        const inspect=async table=>(await action(page,'document.table.inspect',{table,limit:32})).value;
        const layout=async table=>(await action(page,'document.table.layout',{table,limit:8})).value;
        const select=async(paragraph,start=0,end=start)=>action(page,'document.select',{paragraph,start,end});
        const paste=async html=>page.evaluate(text=>{
          const data=new DataTransfer();data.setData('text/plain','HTML-Tabelle');data.setData('text/html',text);
          const input=document.querySelector('#input');input.focus({preventScroll:true});
          const event=new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true});
          if(event.clipboardData!==data)for(const type of data.types)event.clipboardData.setData(type,data.getData(type));input.dispatchEvent(event);
        },html);
        await open();await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'floating-pages.html',mimeType:'text/html',buffer:html});
        await until(()=>state(page),s=>s.totalTables===1&&s.dirty&&s.page.height===5760);
        let d=await state(page),table=d.tables[0].id,position=d.tables[0].position,g=await layout(table);
        assert.equal(g.pageCount,4);assert.ok(g.pages.slice(1).every(p=>p.cells.filter(c=>c.repeated).length===2));
        assert.equal(g.pages.flatMap(p=>p.cells).filter(c=>!c.repeated).length,24);
        // The table, rather than main-body text, creates these additional pages.
        const target=g.pages[1].cells.find(c=>!c.repeated&&c.column===0),paragraph=target.paragraph;
        await select(paragraph);await painted();
        const at=async(index,x,y)=>{const sheet=await widget(0x300000+index);return point(sheet.rect[0]+x*sheet.rect[2]/d.page.width,sheet.rect[1]+y*sheet.rect[2]/d.page.width);};
        // Bring this actual table cell into the canvas viewport before clicking its text.
        for(let tries=0;tries<20;tries++) {
          const pos=await at(1,target.bounds[0]+250,target.bounds[1]+250);
          if(pos[1]>275&&pos[1]<890)break;
          await page.mouse.move(1120,650);await page.mouse.wheel(0,pos[1]>890?400:-400);await painted();
        }
        let pos=await at(1,target.bounds[0]+250,target.bounds[1]+250);assert.ok(pos[1]>240&&pos[1]<950,pos);
        await page.mouse.click(...pos);await page.keyboard.press('End');await page.keyboard.insertText(' · Web');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===paragraph)?.text.includes('Web'));
        await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-continuation.png`)});
        const changed=await exportDoc();await action(page,'document.undo');assert.ok(!(await state(page)).paragraphs.find(p=>p.id===paragraph).text.includes('Web'));await action(page,'document.redo');assert.deepEqual(await exportDoc(),changed);
        // Edit a repeated header instance on page two. All instances share its stable paragraph.
        g=await layout(table);const header=g.pages[1].cells.find(c=>c.repeated&&c.column===0);
        await select(header.paragraph);await painted();
        for(let tries=0;tries<20;tries++){const pos=await at(1,header.bounds[0]+250,header.bounds[1]+250);if(pos[1]>280&&pos[1]<860)break;await page.mouse.move(1120,650);await page.mouse.wheel(0,pos[1]>860?350:-350);await painted();}
        await page.mouse.click(...await at(1,header.bounds[0]+250,header.bounds[1]+250));await page.keyboard.press('Control+End');await page.keyboard.insertText(' H');
        await until(()=>state(page),s=>s.paragraphs.find(p=>p.id===header.paragraph)?.text.endsWith(' H'));
        assert.equal((await layout(table)).pages.filter(p=>p.cells.some(c=>c.id===header.id)).length,4);
        const revision=(await state(page)).revision,markup=(await exportDoc('html')).toString();
        assert.ok(markup.includes('data-jf-table-position="1 margin paragraph 180 270 60 120 90 150 0"'));
        assert.ok(markup.includes('data-jf-page="1 8640 5760 720 720 720 720"'));assert.equal((await state(page)).revision,revision);
        const updated=await exportDoc(),bad=markup.replace('1 margin paragraph 180','2 margin paragraph 180');
        await select(paragraph);await paste(bad);await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label?.includes('table position version or fields')));
        assert.equal((await state(page)).revision,revision);assert.deepEqual(await exportDoc(),updated);
        // External HTML-only paste cannot rely on a private native clipboard payload.
        const anchor=(await state(page)).tables[0].before;await select(anchor);await paste(markup);await until(()=>state(page),s=>s.totalTables===2);
        assert.ok((await state(page)).tables.every(t=>JSON.stringify(t.position)===JSON.stringify(position)));
        await action(page,'document.undo');assert.deepEqual(await exportDoc(),updated);
        // File import preserves page reference coordinates as well as every position field.
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'roundtrip.html',mimeType:'text/html',buffer:Buffer.from(markup)});
        await until(()=>state(page),s=>s.totalTables===1&&s.page.height===5760);d=await state(page);table=d.tables[0].id;
        assert.deepEqual(d.tables[0].position,position);assert.equal((await layout(table)).pageCount,4);
        const path='/documents/floating-pages.docx',task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await writeFile(resolve(output,`${engine}-${theme}-${scale}.html`),markup);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        assert.equal((await layout((await state(page)).tables[0].id)).pageCount,4);
        assert.deepEqual(errors,[]);results.push({engine,theme,scale,compute,pages:4,continuationEditing:true,repeatedHeaderEditing:true,htmlPositionRoundtrip:true,htmlClipboard:true,invalidMetadataAtomic:true,pageSettings:true,durableReload:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: four floating pages, direct continuation/header editing, HTML coordinates, clipboard and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
