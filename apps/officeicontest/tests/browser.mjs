// Compare the same Rust font probe in Chromium/Firefox with its native pixels.
// Run typography_reference.py first; build officeicontest with --target all.
// node apps/officeicontest/tests/browser.mjs build/wordcraft-type
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, writeFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const root=resolve('.'), build=resolve(process.argv[2] || 'build/wordcraft-type');
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve(root,'build/browser-engines');
const playwright=loadPlaywright();
const server=createServer(async (request,response)=>{
  try {
    const path=decodeURIComponent(new URL(request.url,'http://localhost').pathname);
    if(path==='/favicon.ico') {response.writeHead(204).end();return;}
    const native=path.startsWith('/native/');
    const folder=resolve(build,native?'typography':'web/officeicontest');
    const file=resolve(folder,(native?path.slice(8):path.slice(1)) || 'index.html');
    if(!file.startsWith(folder+sep)) throw new Error('Invalid path');
    const bytes=await readFile(file);
    response.writeHead(200,{'content-type':{'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.png':'image/png'}[extname(file)] || 'application/octet-stream'}).end(bytes);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const base=`http://127.0.0.1:${server.address().port}`, results=[];
let nextId=0;
async function rpc(page,method,params={}) {
  const response=await page.evaluate(request=>window.jfguiAgent.request(request),{jsonrpc:'2.0',id:++nextId,method,params});
  assert.ok(!response.error,JSON.stringify(response));
  return response.result;
}
try {
  for(const engine of ['chromium','firefox']) {
    const browser=await playwright[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      for(const scale of [100,150,200]) for(const theme of ['light','dark']) {
        // Browser host bar and the integrated app titlebar each occupy 38 DIP.
        const context=await browser.newContext({viewport:{width:560,height:516},deviceScaleFactor:scale/100});
        try {
          const page=await context.newPage();
          const errors=[];
          page.on('pageerror',e=>errors.push(String(e)));
          await page.addInitScript(()=>{window.__jfguiReady=false;addEventListener('jfgui-ready',()=>window.__jfguiReady=true);});
          await page.goto(`${base}/?agent=1&theme=${theme}`);
          await page.waitForFunction(()=>window.__jfguiReady,{timeout:45000});
          const state=await rpc(page,'state/inspect',{domain:'ui'});
          await rpc(page,'actions/execute',{action:'ui.command',input:{id:3,arg:1},expectedRevision:state.revision});
          await rpc(page,'actions/execute',{action:'ui.render',input:{}});
          const probe=(await rpc(page,'state/inspect',{domain:'ui',limit:100})).widgets.find(w=>w.id===500);
          assert.ok(probe,'Typography canvas is present');
          // Allow the Worker frame and its Canvas presentation to reach the page.
          await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
          const difference=await page.evaluate(async ({url,rect})=>{
            const bitmap=await createImageBitmap(await (await fetch(url)).blob());
            const native=document.createElement('canvas');native.width=bitmap.width;native.height=bitmap.height;
            native.getContext('2d').drawImage(bitmap,0,0);bitmap.close();
            const screen=document.querySelector('#screen');
            if(native.width!==rect[2] || native.height!==rect[3]) return {size:false,native:[native.width,native.height],web:rect};
            const a=native.getContext('2d').getImageData(0,0,native.width,native.height).data;
            const b=screen.getContext('2d').getImageData(...rect).data;
            let pixels=0,maxChannelError=0;
            for(let i=0;i<a.length;i+=4) {
              let error=0;for(let c=0;c<4;c++)error=Math.max(error,Math.abs(a[i+c]-b[i+c]));
              if(error)pixels++;maxChannelError=Math.max(maxChannelError,error);
            }
            return {size:true,pixels,maxChannelError};
          },{url:`${base}/native/win-${scale}-${theme}.png`,rect:probe.rect});
          const frame=await page.evaluate(()=>document.querySelector('#screen').toDataURL('image/png').split(',')[1]);
          await writeFile(resolve(build,`typography/${engine}-${scale}-${theme}.png`),Buffer.from(frame,'base64'));
          assert.deepEqual(difference,{size:true,pixels:0,maxChannelError:0},`${engine}/${scale}/${theme}`);
          assert.deepEqual(errors,[]);
          results.push({engine,scale,theme,scope:'Typography canvas; window chrome excluded',probeRect:probe.rect,...difference});
          console.log(`PASS ${engine} ${scale}% ${theme}: native/Canvas pixels identical`);
        } finally {await context.close();}
      }
    } finally {await browser.close();}
  }
  await writeFile(resolve(build,'typography/browser-results.json'),JSON.stringify(results,null,2));
} finally {await new Promise(resolve=>server.close(resolve));}
