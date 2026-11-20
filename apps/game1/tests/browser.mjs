// game1 in a real browser as a client of game1-server (G7e part 2): the server serves the web build on 127.0.0.1 and
// plays over its WebSocket /game1; the page runs `game1 --client /game1` (src/client.rs) instead of the window:
// clients played by the bots' minds join a room, play, and compare every prediction with the server's state
// (reconciliation); their report lines come through the console.  The page's arguments: a copy of the web build
// whose loader passes them (the loader of web/ takes none from the address).
//
//   python build_rs.py app game1-server                     (the native server)
//   python build_rs.py app game1 --target web
//   node apps/game1/tests/browser.mjs [BUILD_DIR] [--engine=chromium,firefox] [--secs=12] [--clients=2] [--fire]
//
// Playwright through tools/tests/playwright.mjs (PLAYWRIGHT_MODULE, PLAYWRIGHT_BROWSERS_PATH as there); exit code 1
// when a check fails.  The server listens on 127.0.0.1 only.
import {spawn} from 'node:child_process';
import {cpSync, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from '../../../tools/tests/playwright.mjs';

const build=resolve(process.argv[2]&&!process.argv[2].startsWith('--') ? process.argv[2] : 'build');
const options=Object.fromEntries(process.argv.slice(2).filter(v=>v.startsWith('--')).map(v=>{const [k,x]=v.slice(2).split('=');return [k,x??'1'];}));
const engines=(options.engine || 'chromium,firefox').split(',');
const secs=Number(options.secs || 12);
const clients=Number(options.clients || 2);
const exe=join(build, process.platform==='win32' ? 'win/game1-server.exe' : process.platform==='darwin' ? 'mac/game1-server' : 'linux/game1-server');
const web=join(build,'web','game1');
if(!existsSync(exe)) throw new Error(`no ${exe}: python build_rs.py app game1-server`);
if(!existsSync(join(web,'loader.js'))) throw new Error(`no ${web}: python build_rs.py app game1 --target web`);
const playwright=loadPlaywright();

// the copy whose loader passes the client's arguments
const dir=mkdtempSync(join(tmpdir(),'game1-browser-'));
cpSync(web,dir,{recursive:true});
const loader=join(dir,'loader.js');
const src=readFileSync(loader,'utf8');
const args=['--client','/game1','--clients',String(clients),'--secs',String(secs),'--leave',...(options.fire?['--fire']:[])];
const patched=src.replace('const args=[];',`const args=${JSON.stringify(args)};`);
if(patched===src) throw new Error('the loader has no "const args=[];" any more: tests/browser.mjs needs another way to pass the arguments');
writeFileSync(loader,patched);

// the server (no bots unless --fire: without shooting a correction is the network's)
const server=spawn(exe,['--port','0','--log','--web',dir,'--bots',options.fire?'2':'0','--secs',String(engines.length*(secs+60))],{stdio:['ignore','pipe','pipe']});
let serverOut='';
const base=await new Promise((done,fail)=>{
  const timer=setTimeout(()=>fail(new Error(`the server did not start: ${serverOut}`)),20000);
  server.stdout.on('data',d=>{
    serverOut+=d;
    const m=/die Seite auf (http:\/\/[^\s/]+\/)/.exec(serverOut);
    if(m) {clearTimeout(timer);done(m[1]);}
  });
  server.stderr.on('data',d=>serverOut+=d);
  server.on('exit',code=>fail(new Error(`the server ended (${code}): ${serverOut}`)));
});
console.log(`server ${base}`);
let failures=0;
try {
  for(const engine of engines) {
    const browser=await launchWasm(playwright[engine],{headless:true,...(engine==='chromium'?chromiumOptions():{})});
    try {
      const page=await browser.newPage();
      const lines=[];
      let done;
      const finished=new Promise(r=>done=r);
      page.on('console',m=>{
        const t=m.text().trimEnd();
        if(t.startsWith('game1 client')) {lines.push(t);console.log(`[${engine}] ${t}`);if(t.startsWith('game1 clients:'))done();}
        else if(m.type()==='error' || options.verbose) console.log(`[${engine}] console: ${t}`);
      });
      page.on('pageerror',e=>console.log(`[${engine}] page error: ${e.message}`));
      if(options.verbose) page.on('requestfailed',r=>console.log(`[${engine}] request failed: ${r.url()} ${r.failure()?.errorText}`));
      if(options.verbose) page.on('response',r=>{if(r.status()>=400)console.log(`[${engine}] ${r.status()} ${r.url()}`);});
      await page.goto(base);
      const timeout=new Promise(r=>setTimeout(()=>r('timeout'),(secs+90)*1000));
      if(await Promise.race([finished,timeout])==='timeout') {failures++;console.log(`[${engine}] FAIL: no report within ${secs+90} s`);}
      const end=lines.find(l=>l.startsWith('game1 clients:'));
      const each=lines.filter(l=>/^game1 client \d+ \(WebSocket\) \(Playing, Connected\)/.test(l));
      if(!end || !end.endsWith("bestanden") || each.length!==clients) {failures++;console.log(`[${engine}] FAIL: ${each.length} of ${clients} clients played, ${end||'no report'}`);}
      // a moment for the LEAVEs
      await page.waitForTimeout(500);
      console.log(`[${engine}] browser ${await browser.version()}`);
    } finally {await browser.close();}
  }
} finally {
  server.kill();
  await new Promise(r=>{const t=setTimeout(r,3000);server.on('exit',()=>{clearTimeout(t);r();});});
  const tail=serverOut.split('\n').filter(l=>l.trim()).slice(-24).join('\n');
  if(tail) console.log(tail);
  rmSync(dir,{recursive:true,force:true});
}
console.log(failures ? `FAIL: ${failures}` : 'game1 browser: all passed');
process.exit(failures ? 1 : 0);
