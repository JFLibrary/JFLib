// nettest's browser test (G7d): the page of the web build connects to a local netio server (nettest --serve, which
// also serves the page) through plat's WebSocket and the channel over it, echoes and measures; then a desktop client
// (nettest --client) reaches the same server over UDP and the WebSocket.  The page's lines come through its console.
//
//   python build_rs.py app nettest                                   (the native server and client)
//   python build_rs.py app nettest --target web --web-mode scalar
//   node apps/nettest/tests/web.mjs [BUILD_DIR] [--engine=chromium,firefox]
//
// Playwright through tools/tests/playwright.mjs (PLAYWRIGHT_MODULE, PLAYWRIGHT_BROWSERS_PATH as there); exit code 1
// when a check fails.  The server listens on 127.0.0.1 only.
import {spawn, spawnSync} from 'node:child_process';
import {existsSync} from 'node:fs';
import {join, resolve} from 'node:path';
import {loadPlaywright, chromiumOptions, launchWasm} from '../../../tools/tests/playwright.mjs';

const build=resolve(process.argv[2]&&!process.argv[2].startsWith('--') ? process.argv[2] : 'build');
const options=Object.fromEntries(process.argv.slice(2).filter(v=>v.startsWith('--')).map(v=>v.slice(2).split('=')));
const engines=(options.engine || 'chromium').split(',');
const exe=join(build, process.platform==='win32' ? 'win/nettest.exe' : process.platform==='darwin' ? 'mac/nettest' : 'linux/nettest');
const web=join(build,'web','nettest');
if(!existsSync(exe)) throw new Error(`no ${exe}: python build_rs.py app nettest`);
if(!existsSync(join(web,'index.html'))) throw new Error(`no ${web}: python build_rs.py app nettest --target web`);
const playwright=loadPlaywright();

// the server: its first line names the ports
const server=spawn(exe,['--serve',web],{stdio:['ignore','pipe','pipe']});
let serverOut='';
const ports=await new Promise((done,fail)=>{
  const timer=setTimeout(()=>fail(new Error(`the server did not start: ${serverOut}`)),20000);
  server.stdout.on('data',d=>{
    serverOut+=d;
    const m=/nettest serve (\d+) (\d+)/.exec(serverOut);
    if(m) {clearTimeout(timer);done({http:Number(m[1]),udp:Number(m[2])});}
  });
  server.stderr.on('data',d=>serverOut+=d);
  server.on('exit',code=>fail(new Error(`the server ended (${code}): ${serverOut}`)));
});
const base=`http://127.0.0.1:${ports.http}`;
console.log(`server ${base}, UDP ${ports.udp}`);
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
        if(t.startsWith('nettest browser:')) {lines.push(t);console.log(`[${engine}] ${t}`);if(t.includes(' done: '))done();}
        else if(m.type()==='error') console.log(`[${engine}] console: ${t}`);
      });
      page.on('pageerror',e=>console.log(`[${engine}] page error: ${e.message}`));
      await page.goto(`${base}/`);
      const timeout=new Promise(r=>setTimeout(()=>r('timeout'),180000));
      if(await Promise.race([finished,timeout])==='timeout') {failures++;console.log(`[${engine}] FAIL: no end within 180 s`);}
      const fails=lines.filter(l=>l.includes('FAIL'));
      const end=lines.find(l=>l.includes(' done: '));
      if(fails.length || !end || !/ 0 failed/.test(end)) failures++;
      console.log(`[${engine}] browser ${await browser.version()}`);
    } finally {await browser.close();}
  }
  // a desktop client against the same server, over UDP and the WebSocket
  const client=spawnSync(exe,['--client',String(ports.http),'--udp',String(ports.udp)],{encoding:'utf8',timeout:60000});
  process.stdout.write(client.stdout||'');
  if(client.status!==0) {failures++;console.log(`desktop client: exit ${client.status} ${client.stderr||''}`);}
} finally {
  try {await fetch(`${base}/quit`);} catch {}
  await new Promise(r=>{const t=setTimeout(()=>{server.kill();r();},5000);server.on('exit',()=>{clearTimeout(t);r();});});
  const tail=serverOut.split('\n').filter(l=>l.includes('channel')).slice(-6).join('\n');
  if(tail) console.log(tail);
}
console.log(failures ? `FAIL: ${failures}` : 'nettest web: all passed');
process.exit(failures ? 1 : 0);
