// Real WASM pointer/keyboard tests, shared rulers, precise dialog and durable transfer.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile, mkdir} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
const build=resolve(process.argv[2] || 'build/wordcraft-tabs'), folder=resolve(build,'web/jfWord'), output=resolve(build,'tabs-browser');
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
  const original=await readFile(resolve(build,'tabs-tests/win-json.docx'));
  for(const engine of ['chromium','firefox']) {
    const browser=await pw[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try { for(const [scale,theme] of [[1,'light'],[1.5,'dark'],[2,'light']]) {
      const context=await browser.newContext({viewport:{width:1480,height:1040},deviceScaleFactor:scale});
      try {
        const page=await context.newPage(), errors=[];
        page.on('pageerror',e=>errors.push(String(e)));
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const open=async()=>{await page.goto(`${base}/?agent=1&theme=${theme}`);await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});};
        const widget=async id=>(await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.id===id))).widgets.find(w=>w.id===id);
        const point=async(x,y)=>{const c=await page.locator('#screen').boundingBox();return [c.x+x/scale,c.y+y/scale];};
        const click=async id=>{const w=await widget(id);await page.mouse.click(...await point(w.rect[0]+w.rect[2]/2,w.rect[1]+w.rect[3]/2));};
        const select=async(start=0,end=start)=>action(page,'document.select',{paragraph:1,start,end});
        const props=async()=>(await state(page)).paragraphs[0].effectiveParagraphFormat;
        const rulerPoint=async(twips,y=12)=>{
          const sheet=await widget(0x300000), ruler=await widget(2852), doc=await state(page);
          return point(sheet.rect[0]+(doc.page.left+twips)*sheet.rect[2]/doc.page.width,ruler.rect[1]+y*scale);
        };
        const drag=async(from,to,cancel=false)=>{await page.mouse.move(...from);await page.mouse.down();await page.mouse.move(...to,{steps:8});if(cancel)await page.keyboard.press('Escape');await page.mouse.up();};
        await open();
        await page.locator('#files').setInputFiles({name:'tabs-native.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:original});
        await until(()=>state(page),s=>s.path==='/imports/tabs-native.docx');
        assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),original);
        await select(4);await page.keyboard.press('Tab');
        await until(()=>state(page),s=>s.paragraphs[0].text==='Ware\t\t12,50');
        await page.keyboard.press('Control+z');await until(()=>state(page),s=>s.paragraphs[0].text==='Ware\t12,50');
        await select();
        let revision=(await state(page)).revision;
        if (process.env.TABS_TRACE) { console.log('ruler', await widget(2852), 'sheet', await widget(0x300000), 'points', await rulerPoint(360,6), await rulerPoint(1080,6)); await page.screenshot({path:resolve(output,'before-drag.png')}); }
        await drag(await rulerPoint(360,6),await rulerPoint(1080,6));
        if (process.env.TABS_TRACE) { console.log('after focus', (await state(page,'ui')).focus, 'format', await props()); await page.screenshot({path:resolve(output,'after-drag.png')}); }
        await until(props,p=>p.firstLine===360);
        assert.equal((await state(page)).revision,revision+1,'One undo step per drag');
        await page.keyboard.press('Control+z');await until(props,p=>p.firstLine===-360);
        revision=(await state(page)).revision;
        await drag(await rulerPoint(360,6),await rulerPoint(900,6),true);
        assert.equal((await state(page)).revision,revision,'Escape cancels');
        // Left rectangle moves both indents together; hanging triangle holds first line.
        await drag(await rulerPoint(720,18),await rulerPoint(1080,18));
        await until(props,p=>p.left===1080 && p.firstLine===-360);
        await drag(await rulerPoint(1080,12),await rulerPoint(1440,12));
        await until(props,p=>p.left===1440 && p.firstLine===-720);
        // Blank ruler click creates a left stop; move it and drag it off to remove it.
        await page.mouse.click(...await rulerPoint(2160));
        await until(props,p=>p.tabs.some(t=>t.position===2160));
        await drag(await rulerPoint(2160),await rulerPoint(2520));
        await until(props,p=>p.tabs.some(t=>t.position===2520) && !p.tabs.some(t=>t.position===2160));
        const from=await rulerPoint(2520);await drag(from,[from[0],from[1]+55]);
        await until(props,p=>!p.tabs.some(t=>t.position===2520));
        // A dropped new tab outside the ruler is cancelled too.
        revision=(await state(page)).revision;
        const newTab=await rulerPoint(2160);await drag(newTab,[newTab[0],newTab[1]+60]);
        assert.equal((await state(page)).revision,revision);
        // Change page margin using the unobstructed top strip.
        await drag(await rulerPoint(0,1),await rulerPoint(180,1));
        await until(()=>state(page),s=>s.page.left===1620);
        const geometry=await widget(0x300000), vertical=await widget(2853), doc=await state(page);
        const topPoint=amount=>point(vertical.rect[0]+11*scale,geometry.rect[1]+amount*geometry.rect[2]/doc.page.width);
        await drag(await topPoint(doc.page.top),await topPoint(1620));
        await until(()=>state(page),s=>s.page.top===1620);
        const width=(await state(page)).page.width-1620-(await state(page)).page.right;
        await drag(await rulerPoint(width,17),await rulerPoint(7200,17));
        await until(props,p=>p.right===width-7200);
        // Document zoom changes the ruler scale but not physical tab positions.
        await action(page,'ui.command',{id:65,arg:150});
        await until(()=>widget(0x300000),w=>w.rect[2]>1100*scale);
        await page.mouse.click(...await rulerPoint(2160));
        await until(props,p=>p.tabs.some(t=>t.position===2160));
        await action(page,'ui.command',{id:65,arg:100});
        await until(()=>widget(0x300000),w=>w.rect[2]<900*scale);
        // Exact point entry and leader choices via the shared dialog, not agent edits.
        await click(2874);await click(858);
        await click(3802);await page.keyboard.press('Control+a');await page.keyboard.insertText('180.05');await page.keyboard.press('Tab');
        const choose=async(id,label)=>{await click(id);const menu=await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label===label && w.id!==id));await click(menu.widgets.find(w=>w.label===label && w.id!==id).id);};
        await choose(3803,'Rechts');await choose(3804,'Punkte');await click(3805);
        await until(()=>state(page,'ui'),s=>s.widgets.some(w=>w.label==='180.05 pt · Rechts · Punkte'));
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-dialog.png`)});
        await click((await state(page,'ui')).widgets.find(w=>w.label==='Übernehmen').id);
        await until(props,p=>p.tabs.some(t=>t.position===3601 && t.alignment==='right' && t.leader==='dot'));
        await page.screenshot({path:resolve(output,`${engine}-${theme}-${scale}-page.png`)});
        const path='/documents/tabs-browser.docx';const task=await action(page,'file.save',{path});
        await until(()=>rpc(page,'tasks/get',{id:task.task}),s=>s.status==='complete' && s.storage==='durable');
        const bytes=await resource(page,(await action(page,'document.export')).resources[0].uri);
        await writeFile(resolve(output,`${engine}-${theme}-${scale}.docx`),bytes);
        await open();await action(page,'file.open',{path});
        assert.deepEqual(await resource(page,(await action(page,'document.export')).resources[0].uri),bytes);
        assert.deepEqual(errors,[]);
        results.push({engine,theme,scale,bytes:bytes.length,rulerGestures:true,undo:true,preciseDialog:true,durableReload:true});
        console.log(`PASS ${engine} ${theme} ${scale}: rulers, tab stops, native transfer, undo, dialog and persistence`);
      } finally {await context.close();}
    } } finally {await browser.close();}
  }
  await writeFile(resolve(output,'results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(r=>server.close(r));}
