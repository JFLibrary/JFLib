// Nested document blocks, external clipboard MIME types and file import in real WASM.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-nested-tables'), folder=resolve(build,'web/jfWord'), output=resolve(build,'nested-tables-browser');
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
  const html=await readFile(resolve('apps/jfWord/fixtures/nested-tables.html'),'utf8');
  const rtf=await readFile(resolve('apps/jfWord/fixtures/nested-tables.rtf'),'utf8');
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
        const select=async(paragraph,start=0,end=start)=>action(page,'document.select',{paragraph,start,end});
        const paste=async(format,text,plain=false)=>page.evaluate(({format,text,plain})=>{
          const data=new DataTransfer();data.setData('text/plain','externes Tabellenfragment');if(format!=='text/plain')data.setData(format,text);else data.setData(format,text);
          const input=document.querySelector('#input');input.focus({preventScroll:true});
          const event=new ClipboardEvent('paste',{clipboardData:data,bubbles:true,cancelable:true});
          // Firefox allocates its own writable DataTransfer for synthetic ClipboardEvents.
          if(event.clipboardData!==data)for(const type of data.types)event.clipboardData.setData(type,data.getData(type));
          if(event.clipboardData.getData(format)!==text)throw Error('Browser discarded synthetic clipboard MIME data');
          if(plain)input.dispatchEvent(new KeyboardEvent('keydown',{key:'v',ctrlKey:true,shiftKey:true,bubbles:true}));
          input.dispatchEvent(event);
          if(plain)input.dispatchEvent(new KeyboardEvent('keyup',{key:'v',ctrlKey:true,shiftKey:true,bubbles:true}));
        },{format,text,plain});
        await open();await action(page,'document.new',{discardChanges:true});await select(1);
        // HTML arrives through the actual DOM paste adapter; codecs run in the worker.
        await paste('text/html',html);await until(()=>state(page),s=>s.totalTables===3);
        let d=await state(page),root=d.tables[0].id,leaf=d.tables.find(t=>t.depth===3).id;
        let p=(await inspect(leaf)).rows[0].cells[0].paragraphs[0];
        const initial=await exportDoc();await action(page,'document.undo');assert.equal((await state(page)).totalTables,0);await action(page,'document.redo');assert.deepEqual(await exportDoc(),initial);
        // Direct canvas hit on the nested cell must edit the deepest story.
        const layout=(await action(page,'document.table.layout',{table:leaf})).value;
        const cell=layout.pages[0].cells[0],sheet=await widget(0x300000);
        await page.mouse.click(...await point(sheet.rect[0]+(cell.bounds[0]+cell.bounds[2]/2)*sheet.rect[2]/d.page.width,sheet.rect[1]+(cell.bounds[1]+cell.bounds[3]/2)*sheet.rect[2]/d.page.width));
        await page.keyboard.press('End');await page.keyboard.insertText(' · Web');
        await until(()=>state(page),s=>s.paragraphs.find(v=>v.id===p)?.text.includes('Web'));
        await select(p);await painted();await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-nested.png`)});
        // Insert a new nested table using the same ribbon/dialog as native jfWord.
        const parent=(await inspect(root)).rows[1].cells[1].paragraphs[0];await select(parent);await click(2003);await click(1514);
        await set(3901,'2');await set(3902,'1');await label('Übernehmen');await until(()=>state(page),s=>s.totalTables===4);
        await page.keyboard.insertText('Neue innere Tabelle');await until(()=>state(page),s=>s.paragraphs.some(p=>p.text==='Neue innere Tabelle'));
        const beforeCut=await exportDoc();await click(3967);await until(()=>state(page),s=>s.totalTables===3);await action(page,'document.undo');assert.deepEqual(await exportDoc(),beforeCut);
        // A foreign RTF event replaces the cached private DOCX instead of pasting stale data.
        await select(1);await paste('text/rtf',rtf);await until(()=>state(page),s=>s.totalTables===6);
        const beforePlain=(await state(page)).totalTables;await paste('text/plain','Nur Text');await until(()=>state(page),s=>s.paragraphs.some(p=>p.text.includes('Nur Text')));assert.equal((await state(page)).totalTables,beforePlain);
        // Explicit Ctrl+Shift+V ignores unsupported rich content and uses the supplied text.
        await paste('text/html','<p><img src="about:invalid"></p>',true);await until(()=>state(page),s=>s.paragraphs.some(p=>p.text.includes('externes Tabellenfragment')));assert.equal((await state(page)).totalTables,beforePlain);
        const path='/documents/nested-tables.docx',task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete'&&s.storage==='durable');
        const bytes=await exportDoc();await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});assert.deepEqual(await exportDoc(),bytes);
        // File input uses the same shared HTML/RTF decoders and leaves an unsaved DOCX.
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'nested.rtf',mimeType:'application/rtf',buffer:Buffer.from(rtf)});
        await until(()=>state(page),s=>s.totalTables===2&&s.dirty&&s.path==='');
        await action(page,'document.new',{discardChanges:true});
        await page.locator('#files').setInputFiles({name:'nested.html',mimeType:'text/html',buffer:Buffer.from(html)});
        await until(()=>state(page),s=>s.totalTables===3&&s.dirty&&s.path==='');
        await page.setViewportSize({width:680,height:800});d=await state(page);leaf=d.tables.find(t=>t.depth===3).id;p=(await inspect(leaf)).rows[0].cells[0].paragraphs[0];await select(p);await painted();
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-compact.png`)});
        assert.deepEqual(errors,[]);results.push({engine,theme,scale,compute,externalHtmlPaste:true,externalRtfPaste:true,staleClipboardCleared:true,explicitPlainPaste:true,canvasEditing:true,nestedDialog:true,undoSubtree:true,durableReload:true,fileImport:true,bytes:bytes.length});
        console.log(`PASS ${engine} ${theme} ${scale}: nested canvas, dialog, external HTML/RTF clipboard, file import, undo and durable DOCX`);
      }finally{await context.close();}
    }}finally{await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
}finally{await new Promise(r=>server.close(r));}
