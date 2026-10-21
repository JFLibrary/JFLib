// Real WASM character-format roundtrips, native DOCX transfer and dialog input at two DPIs.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-character'), folder=resolve(build,'web/jfWord'), output=resolve(build,'character-browser');
await mkdir(output,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve('build/browser-engines');
const pw=loadPlaywright();
const server=createServer(async(req,res)=>{
  try {
    let name=decodeURIComponent(new URL(req.url,'http://localhost').pathname).slice(1) || 'index.html';
    if(name==='favicon.ico'){res.writeHead(204).end();return;}
    const path=resolve(folder,name);if(!path.startsWith(folder+sep))throw new Error('Invalid path');
    res.writeHead(200,{'content-type':{'.js':'text/javascript','.html':'text/html','.css':'text/css','.wasm':'application/wasm'}[extname(path)] || 'application/octet-stream','cache-control':'no-store'}).end(await readFile(path));
  } catch(e){res.writeHead(404).end(String(e));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const base=`http://127.0.0.1:${server.address().port}`, results=[];
let serial=0;
async function rpc(page,method,params={}){
  const response=await page.evaluate(r=>window.jfguiAgent.request(r),{jsonrpc:'2.0',id:++serial,method,params});
  assert.ok(!response.error,JSON.stringify(response));return response.result;
}
const state=(page,domain='document')=>rpc(page,'state/inspect',{domain,limit:domain==='ui'?500:100});
async function action(page,action,input={}){return rpc(page,'actions/execute',{action,input,expectedRevision:(await state(page,action.startsWith('ui.')?'ui':'document')).revision});}
async function until(fn,ok){const end=Date.now()+15000;let last;do{last=await fn();if(ok(last))return last;await new Promise(r=>setTimeout(r,70));}while(Date.now()<end);throw new Error(JSON.stringify(last));}
async function resource(page,uri){let offset=0;const data=[];for(;;){const p=await rpc(page,'resources/read',{uri,offset,limit:65536});data.push(Buffer.from(p.data,'base64'));if(p.eof)return Buffer.concat(data);offset=p.nextOffset;}}
try {
  const original=await readFile(resolve(build,'character-tests/win-json.docx'));
  for(const engine of ['chromium','firefox']){
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const [scale,theme] of [[1,'light'],[1.5,'dark']]){
        const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
        try {
          const page=await context.newPage(), errors=[];
          page.on('pageerror',e=>errors.push(String(e)));
          await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
          const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
          const click=async id=>{
            const ui=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id)), w=ui.widgets.find(w=>w.id===id);
            const canvas=await page.locator('#screen').boundingBox();
            await page.mouse.click(canvas.x+(w.rect[0]+w.rect[2]/2)/scale,canvas.y+(w.rect[1]+w.rect[3]/2)/scale);
          };
          const select=async(start,end)=>action(page,'document.select',{paragraph:1,start,end});
          const format=async(start,end,summary=false)=>(await action(page,summary?'document.text.format.summary':'document.text.format.inspect',{paragraph:1,start,end})).value;
          await open();
          await page.locator('#files').setInputFiles({name:'characters-native.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
          await until(()=>state(page),s=>s.path==='/imports/characters-native.docx');
          assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),original,'Exact native to WASM transfer');
          await select(1,2);
          assert.equal((await format(1,2)).positioning,'subscript');
          await click(2889);
          await until(()=>format(1,2),f=>f.positioning==='superscript');
          await click(2889);
          await until(()=>format(1,2),f=>f.positioning==='baseline');
          await click(2888);
          await until(()=>format(1,2),f=>f.positioning==='subscript');
          await click(2894);await click(3702); // Green
          await until(()=>format(1,2),f=>f.highlight==='green');
          await click(2894);await click(3700); // Explicit clear
          await until(()=>format(1,2),f=>f.highlight==='none');
          await page.keyboard.press('Control+z');
          await until(()=>format(1,2),f=>f.highlight==='green');
          await click(2894);await click(3701); // Yellow
          await until(()=>format(1,2),f=>f.highlight==='yellow');
          await select(0,8);
          assert.equal((await format(0,8,true)).positioning,null);
          await click(2889);
          await until(()=>format(0,8,true),f=>f.positioning==='superscript');
          await page.keyboard.press('Control+z');
          await until(()=>format(0,8,true),f=>f.positioning===null);
          // Real DOM text entry into a collapsed, inherited script run, then undo.
          await select(8,8);
          await page.keyboard.insertText('β');
          await until(()=>state(page),s=>s.paragraphs[0].text==='H2O + x2β');
          assert.equal((await format(8,10)).positioning,'superscript');
          await page.keyboard.press('Control+z');
          await until(()=>state(page),s=>s.paragraphs[0].text==='H2O + x2');
          // Moving the selection does not alter document formatting or export bytes.
          const before=await resource(page,(await action(page,'document.export')).resources[0].uri);
          await action(page,'document.select',{paragraph:1,start:9,end:9});
          await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
          assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),before);
          // Edit both properties through actual dialog controls, then undo the style edit.
          await click(2881);
          await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===3500));
          const choose=async(id,label)=>{
            await click(id);
            const matches=w=>w.label===label && ![2888,2889].includes(w.id);
            const menu=await until(()=>state(page,'ui'),s=>s.widgets.some(matches));
            const option=menu.widgets.find(matches);
            await click(option.id);
            await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id && w.label===label));
          };
          await choose(3527,'Rot');
          await choose(3528,'Tiefgestellt');
          await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-style.png`)});
          const save=(await state(page,'ui')).widgets.find(w=>w.label==='Speichern' && w.id!==3);
          assert.ok(save);await click(save.id);
          await until(()=>format(10,11),f=>f.positioning==='subscript' && f.highlight==='red');
          assert.equal((await format(9,10)).positioning,'baseline','Direct clear survives the style change');
          await select(9,9);await page.keyboard.press('Control+z');
          await until(()=>format(10,11),f=>f.positioning==='superscript' && f.highlight==='green');
          const path='/documents/character-browser.docx';
          const task=await action(page,'file.save',{path});
          await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
          await until(()=>state(page),s=>!s.dirty);
          const bytes=await resource(page,(await action(page,'document.export')).resources[0].uri);
          await writeFile(resolve(output,`${engine}-${theme}.docx`),bytes);
          await open();await action(page,'file.open',{path});
          assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),bytes);
          assert.equal((await format(1,2)).positioning,'subscript');
          assert.equal((await format(1,2)).highlight,'yellow');
          assert.deepEqual(errors,[]);
          results.push({engine,scale,theme,bytes:bytes.length,nativeTransferExact:true,pointerInput:true,textInput:true,durableReload:true});
          console.log(`PASS ${engine} ${theme} ${scale}: native DOCX, real ribbon input, scripts, highlight, persistence`);
        } finally {await context.close();}
      }
    } finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
