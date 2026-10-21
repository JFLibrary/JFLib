// Real WASM font/style roundtrips, native DOCX transfer and dialog input at two DPIs.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-styles'), folder=resolve(build,'web/jfWord'), output=resolve(build,'style-browser');
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
  const original=await readFile(resolve('build/style-checks/win-json.docx'));
  for(const engine of ['chromium','firefox']){
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const [scale,theme] of [[1,'light'],[1.5,'dark']]){
        const context=await browser.newContext({viewport:{width:1400,height:938},deviceScaleFactor:scale});
        try {
          const page=await context.newPage(), errors=[];
          page.on('pageerror',e=>errors.push(String(e)));
          await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
          const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
          await open();
          await page.locator('#files').setInputFiles({name:'styles-native.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
          await until(()=>state(page),s=>s.path==='/imports/styles-native.docx');
          const selected={paragraph:1,start:0,end:Buffer.byteLength('Wissenschaft αβ')};
          let format=(await action(page,'document.text.format.inspect',selected)).value;
          assert.equal(format.fontFamily,'DejaVu Sans Mono');assert.equal(format.sizeHalfPoints,31);
          let exported=await action(page,'document.export');
          assert.deepEqual(await resource(page,exported.resources[0].uri),original,'Native to WASM: exact DOCX bytes');
          await action(page,'document.text.format.reset',selected);
          await action(page,'document.styles.put',{id:'jfStyle1',name:'Wissenschaft',basedOn:'Normal',format:{fontFamily:'DejaVu Serif',sizeHalfPoints:34},paragraph:{after:240}});
          format=(await action(page,'document.text.format.inspect',selected)).value;
          assert.equal(format.fontFamily,'DejaVu Serif');assert.equal(format.sizeHalfPoints,34);assert.equal(format.bold,true);
          await action(page,'document.select',selected);
          await action(page,'ui.click',{id:2881});
          await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===3500));
          // Real pointer/DOM keyboard events edit the style name in the native canvas dialog.
          const ui=await state(page,'ui'), name=ui.widgets.find(w=>w.id===3503);
          assert.ok(name);
          const canvas=await page.locator('#screen').boundingBox();
          await page.mouse.click(canvas.x+(name.rect[0]+name.rect[2]/2)/scale,canvas.y+(name.rect[1]+name.rect[3]/2)/scale);
          await page.keyboard.press('Control+a');await page.keyboard.insertText('Forschungsfrage Web');
          await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===3503 && w.label==='Forschungsfrage Web'));
          await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
          await action(page,'ui.click',{id:3552});
          const paragraphUi=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===3524));
          const scroll=paragraphUi.widgets.find(w=>w.id===3523);
          await page.mouse.move(canvas.x+(scroll.rect[0]+scroll.rect[2]/2)/scale,canvas.y+(scroll.rect[1]+scroll.rect[3]/2)/scale);
          await state(page,'ui');
          await page.mouse.wheel(0,300);
          await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===3522 && w.value>0));
          await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-paragraph.png`)});
          const save=(await state(page,'ui')).widgets.find(w=>w.label==='Speichern' && w.id!==3);
          assert.ok(save);await action(page,'ui.click',{id:save.id});
          const styles=(await action(page,'document.styles.list')).value.styles;
          assert.equal(styles.find(s=>s.id==='jfStyle2').name,'Forschungsfrage Web');
          assert.equal(styles.find(s=>s.id==='jfStyle2').basedOn,'jfStyle1');
          const path='/documents/style-browser.docx';
          const task=await action(page,'file.save',{path});
          await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
          await until(()=>state(page),s=>!s.dirty);
          exported=await action(page,'document.export');
          const bytes=await resource(page,exported.resources[0].uri);
          await writeFile(resolve(output,`${engine}-${theme}.docx`),bytes);
          await open();await action(page,'file.open',{path});
          assert.deepEqual((await action(page,'document.text.format.inspect',selected)).value,format);
          await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
          assert.deepEqual(errors,[]);
          results.push({engine,scale,theme,bytes:bytes.length,nativeTransferExact:true,inheritance:true,durableReload:true});
          console.log(`PASS ${engine} ${theme} ${scale}: native DOCX, styles, dialog, fonts, persistence`);
        } finally {await context.close();}
      }
    } finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
