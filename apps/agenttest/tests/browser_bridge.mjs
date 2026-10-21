// Real CLI -> authenticated loopback bridge -> browser worker -> Rust dispatcher test.
// node apps/agenttest/tests/browser_bridge.mjs build/portable-work [chromium|firefox]
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile} from 'node:fs/promises';
import {resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';
import {spawn} from 'node:child_process';

const root=resolve('.'), build=resolve(process.argv[2] || 'build/portable-work');
const engine=process.argv[3] || 'chromium';
process.env.PLAYWRIGHT_BROWSERS_PATH ||= resolve(root,'build/browser-engines');
const playwright=loadPlaywright();
const children=new Set();
function child(args) {
  const process=spawn('python',[resolve(root,'tools/jfgui-agent.py'),...args],{cwd:root,windowsHide:true,stdio:'pipe'});
  children.add(process);process.on('close',()=>children.delete(process));return process;
}
async function cli(args,input='') {
  const process=child(args);let stdout='',stderr='';
  process.stdout.on('data',b=>stdout+=b);process.stderr.on('data',b=>stderr+=b);
  const timer=setTimeout(()=>process.kill(),40000);
  process.stdin.end(input);
  const code=await new Promise((resolve,reject)=>{process.once('error',reject);process.once('close',resolve);});
  clearTimeout(timer);return {code,stdout,stderr};
}
const server=createServer(async (request,response)=>{
  try {
    const parts=new URL(request.url,'http://localhost').pathname.split('/').filter(Boolean);
    if(parts.length===1)parts.push('index.html');
    const common=['loader.js','worker.js','host.css','bridge.js','storage.js','a11y.js'];
    const filename=parts.length===2 && common.includes(parts[1]) ? resolve(root,'web',parts[1]) : resolve(build,'web',...parts);
    if(!filename.startsWith(resolve(build,'web')+sep) && !filename.startsWith(resolve(root,'web')+sep))throw new Error('Invalid path');
    const bytes=await readFile(filename);
    response.writeHead(200,{'content-type':{'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm'}[extname(filename)]||'application/octet-stream','cache-control':'no-store'}).end(bytes);
  }catch(error){response.writeHead(404).end(String(error));}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}`;
let browser;
let checks=0;
try {
  const process=child(['browser-bridge','--origin',origin]);
  const config=await new Promise((resolve,reject)=>{
    let text='';const timer=setTimeout(()=>reject(new Error('Bridge did not start')),10000);
    process.once('error',reject);
    process.stderr.on('data',b=>{text+=b;if(text.includes('\n')){clearTimeout(timer);try{resolve(JSON.parse(text.split('\n')[0]));}catch(error){reject(error);}}});
  });
  browser=await playwright[engine].launch({headless:true,...(engine==='chromium'?chromiumOptions():{})});
  const page=await browser.newPage({viewport:{width:1200,height:800}});
  await page.addInitScript(()=>window.addEventListener('jfgui-ready',()=>{window.__jfguiReady=true;},{once:true}));
  const errors=[];page.on('pageerror',error=>errors.push(String(error)));
  page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
  await page.goto(origin+'/jfWord/');
  try {await page.waitForFunction(()=>window.__jfguiReady,null,{timeout:45000});}
  catch(error){throw new Error(`${error.message}; status=${await page.locator('#status').textContent()}; errors=${JSON.stringify(errors)}`);}
  assert.equal(await page.evaluate(()=>typeof window.jfguiAgent),'undefined');checks++;
  await page.locator('#control').click();
  await page.waitForFunction(()=>window.jfguiAgent);
  await page.evaluate(config=>window.jfguiAgent.connectBridge(config).then(()=>true),config);checks++;
  const common=['--url',config.url,'--token',config.token];
  async function rpc(method,params={},session='bridge-json') {
    const result=await cli(['browser-call',...common,'--session',session,method,'--params-json',JSON.stringify(params)]);
    assert.equal(result.code,0,result.stderr||result.stdout);
    const response=JSON.parse(result.stdout);assert.equal(response.id,1);assert.ok(!response.error,JSON.stringify(response));return response.result;
  }
  const document=await rpc('state/inspect',{domain:'document'});
  const paragraph=document.paragraphs[0].id;
  const inserted=await rpc('actions/execute',{action:'document.text.replace',input:{paragraph,start:0,end:0,text:'CLI bis Browser: Grüße 研究'},expectedRevision:document.revision});
  assert.ok(inserted.revision>document.revision);checks++;
  const inspected=await rpc('state/inspect',{domain:'document'});
  assert.equal(inspected.paragraphs[0].text,'CLI bis Browser: Grüße 研究');checks++;
  const meta={'io.modelcontextprotocol/protocolVersion':'2026-07-28','io.modelcontextprotocol/clientCapabilities':{}};
  const modern=await cli(['browser-stdio',...common,'--session','bridge-mcp'],JSON.stringify({jsonrpc:'2.0',id:'catalog',method:'tools/list',params:{_meta:meta}})+'\n');
  assert.equal(modern.code,0,modern.stderr);assert.equal(JSON.parse(modern.stdout).result.tools.length,9);checks++;
  const legacy=[
    {jsonrpc:'2.0',id:'init',method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'bridge-test',version:'1'}}},
    {jsonrpc:'2.0',method:'notifications/initialized'},
    {jsonrpc:'2.0',id:'catalog',method:'tools/list'},
  ].map(v=>JSON.stringify(v)).join('\n')+'\n';
  const initialized=await cli(['browser-stdio',...common,'--session','bridge-legacy'],legacy);
  assert.equal(initialized.code,0,initialized.stderr);const replies=initialized.stdout.trim().split('\n').map(v=>JSON.parse(v));
  assert.equal(replies.length,2);assert.equal(replies[0].result.protocolVersion,'2025-11-25');assert.equal(replies[1].result.tools.length,9);checks++;
  // EOF closes negotiated legacy state; the same explicit session can now start JSON.
  assert.equal((await rpc('agent/info',{},'bridge-legacy')).protocolVersion,'1');checks++;
  for(let i=0;i<20;i++) {
    const result=await cli(['browser-call',...common,'agent/info']);
    assert.equal(result.code,0,result.stderr||result.stdout);assert.equal(JSON.parse(result.stdout).result.protocolVersion,'1');
  }
  checks++;
  const sessionChecks=await page.evaluate(async()=>{
    const agent=window.jfguiAgent;
    for(let i=0;i<24;i++) {
      const response=await agent.request({jsonrpc:'2.0',id:i,method:'agent/info'},{session:`transient-${i}`});
      if(response?.id!==i || response.error)throw new Error('Transient session was not evicted correctly');
    }
    // Preserve initialized legacy sessions even under pressure from stateless clients.
    for(let i=0;i<16;i++) {
      const session=`pinned-${i}`;
      const response=await agent.request({jsonrpc:'2.0',id:i,method:'initialize',params:{protocolVersion:'2025-11-25',capabilities:{},clientInfo:{name:'test',version:'1'}}},{session});
      if(response.error)throw new Error('Legacy initialization failed');
      await agent.request({jsonrpc:'2.0',method:'notifications/initialized'},{session});
    }
    const overflow=await agent.request({jsonrpc:'2.0',id:'overflow',method:'agent/info'},{session:'limit-test'});
    if(overflow.id!=='overflow' || !overflow.error)throw new Error('Uncorrelated session-limit response');
    const retained=await agent.request({jsonrpc:'2.0',id:99,method:'tools/list'},{session:'pinned-0'});
    if(retained.error || retained.result.tools.length!==9)throw new Error('Live legacy session was evicted');
    await agent.closeSession('pinned-0');
    const recovered=await agent.request({jsonrpc:'2.0',id:100,method:'agent/info'},{session:'limit-test'});
    if(recovered.error)throw new Error('Explicit close did not release a session slot');
    for(let i=1;i<16;i++)await agent.closeSession(`pinned-${i}`);
    return true;
  });
  assert.equal(sessionChecks,true);checks+=4;
  const after=await rpc('state/inspect',{domain:'document'});assert.equal(after.revision,inspected.revision);checks++;
  await page.locator('#control').click();
  await page.waitForFunction(()=>typeof window.jfguiAgent==='undefined');
  await new Promise(resolve=>setTimeout(resolve,150));
  const revoked=await cli(['browser-call',...common,'--session','bridge-json','agent/info']);
  assert.equal(revoked.code,1);assert.match(JSON.parse(revoked.stdout).error.message,/No activated browser/);checks++;
  console.log(`browser_bridge (${engine}): ${checks} checks, 0 failures`);
} finally {
  if(browser)await browser.close();
  for(const process of children)process.kill();
  server.closeAllConnections();await new Promise(resolve=>server.close(resolve));
}
