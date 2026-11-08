// lib/render3d in the browser (apps/render3dtest/web.rs) in each of the three bundles: the engine over WebGPU,
// from the same WGSL as on Vulkan.  The program says ok / FAIL per check on the console and sends the gallery's
// images as hex; this test compares them with the images of the Vulkan path on this machine (render3dtest
// --dump, offscreen, both levels) and prints the difference as numbers.  In a browser without a working WebGPU
// it says SKIP.  Playwright: tools/tests/playwright.mjs.
//   python build_rs.py app render3dtest --target win
//   python build_rs.py app render3dtest --target web
//   node apps/render3dtest/tests/engine_web.mjs [build] [--modes=simd,threads,scalar] [--native=FILE]
import {createServer} from 'node:http';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const WIDTH=320, HEIGHT=180, NAMES=['mix','changed','far','tex','gltf'];
// The two paths differ in their shader compilers, so a colour may round the other way; more than this
// is a real difference (the numbers of a run stand in docs/RENDER3D.md)
const MAX_DELTA=Number(option('max-delta','2')), MAX_SHARE=Number(option('max-share','0.01'));

// the Vulkan path on this machine
const here=await mkdtemp(join(tmpdir(),'render3d-web-'));
const native=option('native',resolve('build','win','render3dtest.exe'));
const run=spawnSync(native,['--dump',here],{encoding:'utf8',env:{...process.env,jfgui_TEST_NOACTIVATE:'1'}});
if(run.status!==0) {
  console.log(`FAIL the Vulkan path (${native}): ${run.status}\n${run.stdout}${run.stderr}`);
  process.exit(1);
}
console.log(run.stdout.trim().split('\n').pop());
const skipped=/SKIP/.test(run.stdout);
const reference={};
if(!skipped) for(const level of ['voll','basis']) for(const name of NAMES) reference[`${level}_${name}`]=await readFile(join(here,`${level}_${name}.rgba`));

const pw=loadPlaywright(), dir=resolve(build,'web','render3dtest');
const server=createServer(async (request,response)=>{
  try {
    const parts=decodeURIComponent(new URL(request.url,'http://localhost').pathname).split('/').filter(Boolean);
    if(parts.join('/')==='favicon.ico') {response.writeHead(204).end();return;}
    const file=resolve(dir,...(parts.length ? parts : ['index.html']));
    if(!file.startsWith(dir+sep)) throw new Error('outside');
    const body=await readFile(file);
    const type={'.js':'text/javascript','.css':'text/css','.html':'text/html','.wasm':'application/wasm','.json':'application/json'}[extname(file)] || 'application/octet-stream';
    response.writeHead(200,{'content-type':type,'cache-control':'no-store','cross-origin-opener-policy':'same-origin','cross-origin-embedder-policy':'require-corp'}).end(body);
  } catch(error) {response.writeHead(404).end(String(error));}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));

function compare(a,b) {
  if(a.length!==b.length) return {pixels:Infinity,largest:255,hist:[]};
  let pixels=0, largest=0;
  const hist=[0,0,0,0]; // pixels by their largest difference: 1, 2, 3, more
  for(let i=0;i<a.length;i+=4) {
    const d=Math.max(Math.abs(a[i]-b[i]),Math.abs(a[i+1]-b[i+1]),Math.abs(a[i+2]-b[i+2]),Math.abs(a[i+3]-b[i+3]));
    if(d) {pixels++;largest=Math.max(largest,d);hist[Math.min(d,4)-1]++;}
  }
  return {pixels,largest,hist};
}

let failures=0, ran=0;
try {
  const browser=await pw.chromium.launch({headless:true,...chromiumOptions()});
  try {
    for(const mode of option('modes','simd,threads,scalar').split(',').filter(Boolean)) {
      const page=await browser.newPage(), lines=[];
      let piece='';
      const images={};
      const finished=new Promise(ok=>page.on('console',m=>{
        const t=m.text().trimEnd();
        if(t.startsWith('part ')) {
          if(t==='part end') {
            const [,name,hex]=piece.split(' ');
            images[name]=Buffer.from(hex,'hex');
            piece='';
          } else piece+=t.slice(t.indexOf(' ',5)+1);
          return;
        }
        lines.push(t);
        if(/^render3dtest: |^SKIP render3dtest/.test(t)) ok(t);
      }));
      page.on('pageerror',e=>lines.push(`pageerror ${e}`));
      await page.goto(`http://127.0.0.1:${server.address().port}/index.html?compute=${mode}`);
      const end=await Promise.race([finished,new Promise(ok=>setTimeout(()=>ok('timeout'),90000))]);
      const fail=lines.filter(l=>l.startsWith('FAIL')||l.startsWith('pageerror')||l.startsWith('WebGPU'));
      const count=lines.filter(l=>l.startsWith('ok ')).length;
      const compute=await page.evaluate(()=>window.__jfguiCompute?.mode);
      for(const l of fail) console.log(`${mode}: ${l}`);
      if(args.includes('--verbose')) for(const l of lines) console.log(l);
      if(end.startsWith('SKIP')) console.log(`SKIP ${mode}: ${end}`);
      else {
        ran++;
        let good=end.startsWith('render3dtest: ')&&/ 0 failed/.test(end)&&!fail.length&&count>0&&compute===mode;
        console.log(`${good?'PASS':'FAIL'} chromium ${browser.version()} ${compute}: ${end} (${lines.find(l=>l.startsWith('adapter:'))||'no adapter line'})`);
        for(const name of NAMES) {
          if(!images[name]) {
            console.log(`  FAIL no image ${name}`);
            good=false;
            continue;
          }
          for(const level of skipped?[]:['voll','basis']) {
            const c=compare(images[name],reference[`${level}_${name}`]);
            const fine=c.largest<=MAX_DELTA||c.pixels<=WIDTH*HEIGHT*MAX_SHARE;
            console.log(`  ${fine?'ok  ':'FAIL'} ${name}: against Vulkan ${level}: ${c.pixels} of ${WIDTH*HEIGHT} pixels differ, largest ${c.largest}; by 1 / 2 / 3 / more: ${c.hist.join(' / ')}`);
            good=good&&fine;
          }
        }
        // the same scene at 1e7 draws the same in the browser too
        if(images.mix&&images.far) {
          const c=compare(images.mix,images.far);
          console.log(`  ${c.pixels?'FAIL':'ok  '} at the origin against at 1e7, in the browser: ${c.pixels} pixels differ, largest ${c.largest}`);
          good=good&&!c.pixels;
        }
        // (so that all of this can tell: the images hold something, and the changes show)
        if(images.mix&&images.changed) {
          let colours=new Set(), objects=0;
          for(let i=0;i<images.mix.length;i+=4) {
            colours.add(images.mix[i]<<16|images.mix[i+1]<<8|images.mix[i+2]);
            if(!(images.mix[i]===51&&images.mix[i+1]===102&&images.mix[i+2]===153)) objects++;
          }
          const c=compare(images.mix,images.changed);
          const fine=objects>WIDTH*HEIGHT/4&&colours.size>200&&c.pixels>100;
          console.log(`  ${fine?'ok  ':'FAIL'} the image holds ${objects} pixels of objects in ${colours.size} colours; the changes changed ${c.pixels}`);
          good=good&&fine;
        }
        if(!good) failures++;
      }
      await page.close();
    }
  } finally {await browser.close();}
} finally {
  await new Promise(r=>server.close(r));
  await rm(here,{recursive:true,force:true});
}
if(!ran) console.log('SKIP: no browser here has a working WebGPU');
if(failures) process.exitCode=1;
