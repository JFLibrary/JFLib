// Real-browser acceptance of the common worker/canvas adapter. Needs Playwright (tools/tests/playwright.mjs).
// node tools/tests/web_apps.mjs build/office-universal --engine=chromium --only=template,gallery,jfWord
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, mkdir, writeFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from './playwright.mjs';

const root=resolve('.'), build=resolve(process.argv[2] || 'build/office-universal');
const options=Object.fromEntries(process.argv.slice(3).filter(v=>v.startsWith('--')).map(v=>v.slice(2).split('=')));
const engines=(options.engine || 'chromium').split(','), apps=(options.only || 'template,gallery,jfWord').split(',');
const artifact=resolve(build,'browser-tests');
await mkdir(artifact,{recursive:true});
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve(root,'build/browser-engines');
const playwright=loadPlaywright();
const server=createServer(async (request,response)=>{
  try {
    const url=new URL(request.url,'http://localhost');
    if(url.pathname==='/favicon.ico') {response.writeHead(204).end();return;}
    const parts=decodeURIComponent(url.pathname).split('/').filter(Boolean);
    if(parts.length===1) parts.push('index.html');
    const common=['loader.js','worker.js','host.css','bridge.js','storage.js','secrets.js','a11y.js','audio.js'];
    const filename=parts.length===2 && common.includes(parts[1]) ? resolve(root,'web',parts[1]) : resolve(build,'web',...parts);
    if(!filename.startsWith(resolve(build,'web')+sep) && !filename.startsWith(resolve(root,'web')+sep)) throw new Error('Invalid path');
    const bytes=await readFile(filename);
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.png':'image/png'}[extname(filename)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store',...(options.isolated==='1'?{'cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}:{})}).end(bytes);
  } catch(error) {response.writeHead(404,{'content-type':'text/plain'}).end(String(error));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`;
const results=[];
let requestId=0;
async function raw(page,method,params={}) {
  return page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++requestId,method,params});
}
async function rpc(page,method,params={}) {
  const response=await raw(page,method,params);
  assert.ok(response && !response.error,`${method}: ${JSON.stringify(response)}`);
  return response.result;
}
async function state(page,domain='ui') {return rpc(page,'state/inspect',{domain,limit:100});}
async function action(page,name,input={},revision) {
  if(revision===undefined) revision=(await state(page,name.startsWith('ui.')?'ui':name.startsWith('image.')?'image':'document')).revision;
  return rpc(page,'actions/execute',{action:name,input,expectedRevision:revision});
}
async function until(fn,predicate,description,timeout=15000) {
  const end=Date.now()+timeout;
  let value;
  do {value=await fn();if(predicate(value)) return value;await new Promise(resolve=>setTimeout(resolve,80));} while(Date.now()<end);
  throw new Error(`Timed out: ${description}; last=${JSON.stringify(value)}`);
}
async function pixels(page) {
  return page.evaluate(()=>{
    const c=document.querySelector('#screen'), b=c.getBoundingClientRect(), p=c.getContext('2d').getImageData(0,0,c.width,c.height).data;
    const colors=new Set();for(let i=0;i<p.length;i+=4*71)colors.add(`${p[i]},${p[i+1]},${p[i+2]}`);
    return {width:c.width,height:c.height,cssWidth:b.width,cssHeight:b.height,dpi:devicePixelRatio,colors:colors.size};
  });
}
async function ready(page,url) {
  await page.goto(url);
  await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
  const frame=await until(()=>pixels(page),v=>v.width>300 && v.colors>5,'nonempty Rust canvas frame');
  assert.equal(frame.width,Math.round(frame.cssWidth*frame.dpi));
  assert.equal(frame.height,Math.round(frame.cssHeight*frame.dpi));
  return frame;
}
async function textEvent(page,value,composition=false) {
  await page.locator('#input').focus();
  if(composition) {
    await page.locator('#input').evaluate((input,value)=>{
      input.dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true,data:''}));
      input.dispatchEvent(new InputEvent('beforeinput',{bubbles:true,cancelable:true,inputType:'insertCompositionText',data:value,isComposing:true}));
      input.dispatchEvent(new CompositionEvent('compositionend',{bubbles:true,data:value}));
    },value);
  } else await page.keyboard.insertText(value);
}
async function templateChecks(page) {
  let inspect=await state(page);
  const button=inspect.widgets.find(w=>w.label==='Klick mich');
  assert.ok(button,'Template button has semantic identity');
  await action(page,'ui.click',{id:button.id},inspect.revision);
  inspect=await state(page);
  assert.ok(inspect.widgets.some(w=>w.label==='Klicks: 1'),JSON.stringify(inspect));
  const c=await page.locator('#screen').boundingBox();
  await page.mouse.click(c.x+(button.rect[0]+button.rect[2]/2),c.y+(button.rect[1]+button.rect[3]/2));
  await until(()=>state(page),v=>v.widgets.some(w=>w.label==='Klicks: 2'),'pointer activation through the canvas');
}
async function galleryChecks(page) {
  const inspect=await state(page);
  assert.ok(inspect.total>20,'Gallery widgets are inspectable');
  const entries=await rpc(page,'actions/list',{query:'ui.',limit:30});
  assert.ok(JSON.stringify(entries).includes('ui.type'));
  const all=await rpc(page,'state/inspect',{domain:'ui',limit:500});
  await writeFile(resolve(artifact,'gallery-widgets.json'),JSON.stringify(all,null,2));
  await page.setViewportSize({width:1200,height:1800});
  await until(()=>pixels(page),v=>v.height===1762,'gallery window section');
  const current=await state(page);
  await action(page,'ui.click',{id:7641},current.revision);
  await until(()=>rpc(page,'state/inspect',{limit:500}),v=>v.widgets.some(w=>w.window!==0),'library dialog window');
  await page.screenshot({path:resolve(artifact,'gallery-dialog.png')});
  await page.locator('#input').focus();
  await page.keyboard.press('Escape');
  await until(()=>rpc(page,'state/inspect',{limit:500}),v=>v.widgets.every(w=>w.window===0),'library dialog Escape close');
}
async function resource(page,uri) {
  const chunks=[];let offset=0;
  for(;;) {
    const result=await rpc(page,'resources/read',{uri,offset,limit:65536});
    assert.equal(result.encoding,'base64');chunks.push(Buffer.from(result.data,'base64'));
    if(result.eof)break;
    assert.ok(result.nextOffset>offset);offset=result.nextOffset;
  }
  return Buffer.concat(chunks);
}
async function renderCheck(page,engine,app) {
  const result=await action(page,'ui.render');
  const png=await resource(page,result.resources[0].uri);
  await writeFile(resolve(artifact,`${engine}-${app}-resource.png`),png);
  const difference=await page.evaluate(async encoded=>{
    const image=await createImageBitmap(await (await fetch('data:image/png;base64,'+encoded)).blob());
    const c=document.createElement('canvas');c.width=image.width;c.height=image.height;
    c.getContext('2d').drawImage(image,0,0);image.close();
    const screen=document.querySelector('#screen');
    if(c.width!==screen.width||c.height!==screen.height)return {size:false,pixels:-1};
    const a=c.getContext('2d').getImageData(0,0,c.width,c.height).data;
    const b=screen.getContext('2d').getImageData(0,0,c.width,c.height).data;
    let pixels=0;for(let i=0;i<a.length;i+=4)if(a[i]!==b[i]||a[i+1]!==b[i+1]||a[i+2]!==b[i+2])pixels++;
    return {size:true,pixels};
  },png.toString('base64'));
  assert.deepEqual(difference,{size:true,pixels:0},'Canvas matches the shared Rust PNG renderer exactly');
}
async function kritzelChecks(page) {
  let image=await state(page,'image');
  assert.ok(image.totalLayers>0 && image.width>0);
  await action(page,'image.canvas.resize',{width:40,height:24},image.revision);
  await action(page,'image.layer.fill',{argb:0xff1862c7});
  const pixel=(await action(page,'image.pixel',{x:3,y:4})).value;
  assert.equal(pixel.argb,0xff1862c7);
  await action(page,'image.layer.add');
  image=await state(page,'image');assert.equal(image.totalLayers,2);
  await action(page,'image.layer.fill',{argb:0xffe96831});
  await action(page,'image.rotate.right');
  image=await state(page,'image');assert.equal(image.width,24);assert.equal(image.height,40);
  await action(page,'image.undo');
  image=await state(page,'image');assert.equal(image.width,40);assert.equal(image.height,24);
  assert.equal((await action(page,'image.pixel',{x:3,y:4})).value.argb,0xffe96831);
}
async function storageRecord(page,path) {
  return page.evaluate(path=>new Promise((resolve,reject)=>{
    const request=indexedDB.open('jfgui-files-v1',1);
    request.onerror=()=>reject(request.error);
    request.onsuccess=()=>{
      const db=request.result, value=db.transaction('files','readonly').objectStore('files').get('jfWord:'+path);
      value.onsuccess=()=>{db.close();resolve(value.result?Array.from(new Uint8Array(value.result.bytes)):null);};
      value.onerror=()=>{db.close();reject(value.error);};
    };
  }),path);
}
async function newWordText(page,text) {
  await action(page,'document.new',{discardChanges:true});
  const document=await state(page,'document'), paragraph=document.paragraphs[0].id;
  await action(page,'document.text.replace',{paragraph,start:0,end:0,text},document.revision);
  return paragraph;
}
async function storageFailureChecks(page,engine) {
  // Only this private test page changes the platform access point. The product has no
  // testing flags. Held transactions really commit; their normal oncomplete callback
  // reaches the unchanged host adapter only after another document edit has occurred.
  await page.evaluate(()=>{
    const transaction=IDBDatabase.prototype.transaction;
    const oncomplete=Object.getOwnPropertyDescriptor(IDBTransaction.prototype,'oncomplete');
    const fault={mode:'none',held:[],restore:()=>{IDBDatabase.prototype.transaction=transaction;}};
    window.__idbFault=fault;
    IDBDatabase.prototype.transaction=function(...args) {
      if(this.name==='jfgui-files-v1' && args[1]==='readwrite') {
        if(fault.mode==='quota')throw new DOMException('Injected test storage quota','QuotaExceededError');
        if(fault.mode==='denied')throw new DOMException('Injected test storage denial','NotAllowedError');
      }
      const result=transaction.apply(this,args);
      if(this.name==='jfgui-files-v1' && args[1]==='readwrite' && fault.mode==='hold') {
        let callback=null;
        Object.defineProperty(result,'oncomplete',{configurable:true,get:()=>callback,set(value) {
          callback=value;
          oncomplete.set.call(result,event=>fault.held.push(()=>value.call(result,event)));
        }});
      }
      return result;
    };
  });
  const cases=[];
  try {
    for(const [fault,storage] of [['quota','quota_exceeded'],['denied','access_denied']]) {
      await newWordText(page,`Ungespeicherter Text ${fault}`);
      const before=await state(page,'document'), path=`/documents/${engine}-${fault}.docx`;
      await page.evaluate(mode=>{window.__idbFault.mode=mode;},fault);
      const save=await action(page,'file.save',{path});
      assert.equal(save.status,'pending');
      const task=await until(()=>rpc(page,'tasks/get',{id:save.task}),v=>v.status==='failed',`${fault} reaches the Rust task`);
      assert.equal(task.storage,storage);
      const after=await state(page,'document');
      assert.equal(after.dirty,true,`${fault} must not clear dirty`);
      assert.deepEqual(after,before,`${fault} must not change document revision, path or contents`);
      assert.equal(await storageRecord(page,path),null,`${fault} must not create a durable record`);
      cases.push({case:fault,status:task.status,storage:task.storage,dirty:after.dirty});
      await page.evaluate(()=>{window.__idbFault.mode='none';});
    }
    for(const replacement of [false,true]) {
      const paragraph=await newWordText(page,'Vorherige Fassung');
      const before=await state(page,'document');
      const snapshot=await resource(page,(await action(page,'document.export')).resources[0].uri);
      await page.evaluate(()=>{window.__idbFault.mode='hold';});
      const path=`/documents/${engine}-${replacement?'replaced':'edited'}-pending.docx`;
      const save=await action(page,'file.save',{path});
      await until(()=>page.evaluate(()=>window.__idbFault.held.length),v=>v===1,'real IDB commit with withheld completion notification');
      assert.equal((await rpc(page,'tasks/get',{id:save.task})).status,'running');
      assert.equal((await state(page,'document')).dirty,true,'commit without callback is not reported saved');
      assert.deepEqual(Buffer.from(await storageRecord(page,path)),snapshot,'the held transaction really wrote the requested document');
      if(replacement)await newWordText(page,'Neues Dokument bleibt ungespeichert');
      else await action(page,'document.text.replace',{paragraph,start:Buffer.byteLength(before.paragraphs[0].text),end:Buffer.byteLength(before.paragraphs[0].text),text:' + neu'});
      const edited=await state(page,'document');
      assert.equal(edited.dirty,true);
      assert.notEqual(edited.revision,before.revision);
      await page.evaluate(()=>{window.__idbFault.mode='none';window.__idbFault.held.shift()();});
      const task=await until(()=>rpc(page,'tasks/get',{id:save.task}),v=>v.status==='complete','delayed save completion reaches Rust');
      assert.equal(task.storage,'durable');
      const after=await state(page,'document');
      assert.deepEqual(after,edited,'old durable acknowledgement must not mark the edited or replacement document saved');
      cases.push({case:replacement?'replacement-before-completion':'edit-before-completion',storage:task.storage,dirty:after.dirty,revision:after.revision});
    }
  } finally {
    await page.evaluate(()=>{window.__idbFault.restore();delete window.__idbFault;});
  }
  await writeFile(resolve(artifact,`${engine}-storage-failures.json`),JSON.stringify(cases,null,2));
  await action(page,'document.new',{discardChanges:true});
}
async function formatShortcutChecks(page) {
  const paragraph=await newWordText(page,'Source Target');
  await action(page,'document.text.format',{paragraph,start:0,end:6,format:{bold:true,italic:true}});
  await action(page,'document.select',{paragraph,start:0,end:6});
  const key=async(key,modifiers)=>{
    await page.locator('#input').focus();
    const prevented=await page.locator('#input').evaluate((input,{key,modifiers})=>{
      const events=['keydown','keyup'].map(type=>new KeyboardEvent(type,{key,bubbles:true,cancelable:true,...modifiers}));
      for(const event of events)input.dispatchEvent(event);
      return events[0].defaultPrevented;
    },{key,modifiers});
    assert.equal(prevented,true,'application format shortcut suppresses the browser default action');
  };
  await key('C',{ctrlKey:true,shiftKey:true});
  await until(()=>state(page),v=>v.commands.find(c=>c.id===81)?.enabled===true,'Ctrl+Shift+C enables applying the captured format');
  await action(page,'document.select',{paragraph,start:7,end:13});
  await key('V',{ctrlKey:true,shiftKey:true,altKey:true});
  const transferred=await until(()=>action(page,'document.text.format.inspect',{paragraph,start:7,end:13}),v=>v.value.bold===true && v.value.italic===true,'Ctrl+Shift+Alt+V transfers the captured format through the host');
  assert.equal(transferred.value.bold,true);
  await key(' ',{ctrlKey:true});
  await until(()=>action(page,'document.text.format.inspect',{paragraph,start:7,end:13}),v=>v.value.bold!==true && v.value.italic!==true,'Ctrl+Space removes character overrides');
  assert.equal((await state(page,'document')).paragraphs[0].text,'Source Target','format shortcuts do not paste or remove text');
  await action(page,'document.new',{discardChanges:true});
}
async function wordChecks(page,engine) {
  let document=await state(page,'document');
  const p=document.paragraphs[0].id;
  await action(page,'document.text.replace',{paragraph:p,start:0,end:0,text:'Browserprüfung '},document.revision);
  const conflict=await raw(page,'actions/execute',{action:'document.text.replace',input:{paragraph:p,start:0,end:0,text:'stale'},expectedRevision:document.revision});
  assert.equal(conflict.error?.data?.code,'revision_conflict',JSON.stringify(conflict));
  document=await state(page,'document');
  const bytes=Buffer.byteLength(document.paragraphs[0].text);
  await action(page,'document.select',{paragraph:p,start:bytes,end:bytes},document.revision);
  await textEvent(page,'über DOM ');
  await until(()=>state(page,'document'),v=>v.paragraphs[0].text.includes('über DOM'),'DOM text input');
  await textEvent(page,'研究',true);
  document=await until(()=>state(page,'document'),v=>v.paragraphs[0].text.endsWith('研究'),'IME composition commit');
  const clipboard=await page.locator('#input').evaluate(input=>{
    const data=new DataTransfer();data.setData('text/plain',' Zwischenablage');
    const event=new ClipboardEvent('paste',{bubbles:true,cancelable:true,clipboardData:data});
    // Firefox intentionally hides DataTransfer data on untrusted synthetic paste.
    // Inject only the event payload, leaving the DOM handler and worker import real;
    // this test never reads or replaces the user's operating-system clipboard.
    if(!event.clipboardData?.getData('text/plain')) Object.defineProperty(event,'clipboardData',{
      value:{getData:type=>type==='text/plain'?' Zwischenablage':''},
    });
    const payload=event.clipboardData?.getData('text/plain');
    input.dispatchEvent(event);
    return payload;
  });
  assert.equal(clipboard,' Zwischenablage','synthetic clipboard event contains the imported text');
  document=await until(()=>state(page,'document'),v=>v.paragraphs[0].text.endsWith(' Zwischenablage'),'browser clipboard import');
  const expectedText=document.paragraphs[0].text;
  await action(page,'document.text.format',{paragraph:p,start:0,end:Buffer.byteLength('Browserprüfung'),format:{bold:true}},document.revision);
  const source=await action(page,'sources.put',{title:'Web und native Quellen',kind:'book',authors:[{family:'Muster',given:'Ada'}],year:'2026'});
  assert.ok(source.value?.id || source.output?.id || JSON.stringify(source).includes('id'));
  const beforeExport=await state(page,'document');
  const exported=await action(page,'document.export');
  const exportedBytes=await resource(page,exported.resources[0].uri);
  const afterExport=await state(page,'document');
  assert.equal(afterExport.revision,beforeExport.revision,'export preserves the document revision');
  assert.equal(afterExport.dirty,true,'export is not a durable save');
  await writeFile(resolve(artifact,`browser-${engine}-export.docx`),exportedBytes);
  const path=`/documents/browser-${engine}.docx`;
  const saved=await action(page,'file.save',{path});
  assert.equal(saved.status,'pending',JSON.stringify(saved));
  const task=saved.task;
  assert.ok(task>0,JSON.stringify(saved));
  await until(()=>rpc(page,'tasks/get',{id:task}),v=>v.status==='complete' && v.storage==='durable','confirmed IndexedDB commit');
  document=await until(()=>state(page,'document'),v=>!v.dirty,'dirty flag clears only after save completion');
  assert.equal(document.path,path);
  const stored=await page.evaluate(async path=>new Promise((resolve,reject)=>{
    const r=indexedDB.open('jfgui-files-v1',1);r.onerror=()=>reject(r.error);r.onsuccess=()=>{
      const t=r.result.transaction('files','readonly').objectStore('files').get('jfWord:'+path);
      t.onsuccess=()=>resolve(Array.from(new Uint8Array(t.result.bytes)));t.onerror=()=>reject(t.error);
    };
  }),path);
  assert.deepEqual(stored.slice(0,2),[80,75],'saved DOCX is an actual ZIP package');
  assert.deepEqual(Buffer.from(stored),exportedBytes,'resource export and durable save encode the identical DOCX package');
  await writeFile(resolve(artifact,`browser-${engine}.docx`),Buffer.from(stored));
  await ready(page,`${base}/jfWord/?agent=1&theme=light`);
  await action(page,'file.open',{path});
  document=await state(page,'document');
  assert.equal(document.paragraphs[0].text,expectedText);
  assert.equal(document.sources[0].title,'Web und native Quellen');
  assert.ok(document.paragraphs[0].inlines.some(i=>i.format.bold===true));
  await action(page,'document.new',{discardChanges:true});
  await page.locator('#files').setInputFiles({name:'imported.docx',mimeType:'application/vnd.openxmlformats-officedocument.wordprocessingml.document',buffer:Buffer.from(stored)});
  document=await until(()=>state(page,'document'),v=>v.path==='/imports/imported.docx','browser file chooser import and normal drop/open workflow');
  assert.equal(document.paragraphs[0].text,expectedText);
}
try {
  for(const engine of engines) {
    const browser=await playwright[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const app of apps) {
        const context=await browser.newContext({viewport:{width:1200,height:800},deviceScaleFactor:1});
        const errors=[], page=await context.newPage();
        page.on('pageerror',e=>errors.push(String(e)));
        page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        try {
          const initial=await ready(page,`${base}/${app}/?theme=light`);
          assert.equal(await page.evaluate(()=>typeof window.jfguiAgent),'undefined','default exposes no control API');
          await page.locator('#control').click();
          await until(()=>raw(page,'agent/info'),v=>v?.result?.controlEnabled===true,'explicit control activation');
          assert.equal((await state(page)).control.enabled,true);
          if(app==='template')await templateChecks(page);
          if(app==='gallery')await galleryChecks(page);
          if(app==='jfWord') {await storageFailureChecks(page,engine);await formatShortcutChecks(page);await wordChecks(page,engine);}
          if(app==='Kritzel')await kritzelChecks(page);
          if(app==='template'||app==='Kritzel')await renderCheck(page,engine,app);
          await page.setViewportSize({width:980,height:700});
          const resized=await until(()=>pixels(page),v=>v.width===980 && v.height===662,'host resize to software framebuffer');
          await page.screenshot({path:resolve(artifact,`${engine}-${app}.png`)});
          const badge=(await state(page)).widgets.find(w=>w.id===0xffff7440);
          assert.ok(badge,'shared titlebar shows enabled agent control');
          await page.evaluate(()=>{window.__oldAgent=window.jfguiAgent;});
          const canvas=await page.locator('#screen').boundingBox();
          await page.mouse.click(canvas.x+badge.rect[0]+badge.rect[2]/2,canvas.y+badge.rect[1]+badge.rect[3]/2);
          await until(()=>page.evaluate(()=>typeof window.jfguiAgent),v=>v==='undefined','in-app agent revocation removes browser API');
          assert.equal(await page.evaluate(()=>window.__oldAgent.request({jsonrpc:'2.0',id:1,method:'agent/info'}).then(()=>false,()=>true)),true,'retained API handle is revoked');
          await page.locator('#control').click();
          await until(()=>raw(page,'agent/info'),v=>v?.result?.controlEnabled===true,'explicit reactivation after in-app revocation');
          await page.locator('#control').click();
          assert.equal(await page.evaluate(()=>typeof window.jfguiAgent),'undefined','control revocation removes the API');
          await ready(page,`${base}/${app}/?agent=1&theme=dark`);
          assert.equal((await rpc(page,'agent/info')).controlEnabled,true,'URL opt-in');
          await page.screenshot({path:resolve(artifact,`${engine}-${app}-dark.png`)});
          assert.deepEqual(errors,[]);
          results.push({engine,app,ok:true,initial,resized});
          console.log(`${engine} ${app}: passed`);
        } catch(error) {
          await page.screenshot({path:resolve(artifact,`${engine}-${app}-failure.png`)}).catch(()=>{});
          const status=await page.locator('#status').textContent().catch(()=>null);
          results.push({engine,app,ok:false,error:String(error.stack||error),status,errors});
          console.error(`${engine} ${app}: ${error.stack||error}; status=${status}; console=${errors}`);
        } finally {await context.close();}
      }
      for(const scale of [1.5,2]) for(const app of [apps[0],...apps.filter(a=>a==='jfWord' && a!==apps[0])]) {
        const context=await browser.newContext({viewport:{width:1000,height:760},deviceScaleFactor:scale});
        const page=await context.newPage();
        // Firefox may reset Playwright's DPR emulation on COOP process changes.
        // Supply the requested host DPR explicitly for the adapter's DPI cases.
        if(engine==='firefox' && options.isolated==='1')await page.addInitScript(scale=>{
          if(devicePixelRatio!==scale){Object.defineProperty(window,'devicePixelRatio',{get:()=>scale});window.__jfguiDpiEmulated=true;}
        },scale);
        await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
        const frame=await ready(page,`${base}/${app}/?agent=1&theme=light`);
        assert.equal(frame.width,Math.round(1000*scale));assert.equal(frame.height,Math.round(722*scale));
        await page.screenshot({path:resolve(artifact,`${engine}-${app}-dpi${scale*100}.png`)});
        results.push({engine,app,dpi:scale*100,hostDprOverride:await page.evaluate(()=>!!window.__jfguiDpiEmulated),ok:true,frame});
        await context.close();
      }
    } finally {await browser.close();}
  }
} finally {
  await writeFile(resolve(artifact,'results.json'),JSON.stringify(results,null,2));
  await new Promise(resolve=>server.close(resolve));
}
if(results.some(r=>!r.ok))process.exitCode=1;
