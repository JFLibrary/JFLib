// lib/render2d in the browser (apps/render2dtest/web.rs) in each of the three bundles: the renderer over WebGPU, from the
// same WGSL as on Vulkan.  The program says ok / FAIL per check on the console and sends the images as hex; this test
// compares them with the images of the Vulkan path on this machine (render2dtest --dump, offscreen, both levels) and
// prints the difference as numbers.  The coordinates (the same drawing at the origin and at 1e7, the deep zoom to 1e-6
// units per pixel) and the visibility are judged in the browser, by the code of the Vulkan tests (gallery.rs).  In a
// browser without a working WebGPU it says SKIP.  Playwright: tools/tests/playwright.mjs.
//   python build_rs.py app render2dtest --target win
//   python build_rs.py app render2dtest --target web
//   node apps/render2dtest/tests/engine_web.mjs [build] [--modes=simd,threads,scalar] [--native=FILE] [--gpu=d3d12|vulkan|swiftshader]
//   node apps/render2dtest/tests/engine_web.mjs --bench [--n=1000000] [--thin] [--plain | --g6a2b] [--modes=simd]    the measuring scene (--plain:
//        without arcs and dashes, the drawing of G6a2a; --g6a2b: with arcs and dashes, without fills, hatches and curves)
import {createServer} from 'node:http';
import {mkdtemp, readFile, rm, readdir} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join, resolve, extname, sep} from 'node:path';
import {loadPlaywright, chromiumOptions} from '../../../tools/tests/playwright.mjs';

const args=process.argv.slice(2), build=resolve(args.find(a=>!a.startsWith('--')) || 'build');
const option=(name,fallback)=>args.find(a=>a.startsWith(`--${name}=`))?.split('=')[1] ?? fallback;
const benchRun=args.includes('--bench');
// Which differences are allowed: both paths compute the same numbers with other compilers, a code may round the other way.
const MAX_DELTA=Number(option('max-delta','2')), MAX_SHARE=Number(option('max-share','0.002'));
// Chromium's own choice, or one asked for
const GPU_FLAGS={d3d12:['--use-webgpu-adapter=d3d12'],vulkan:['--enable-features=Vulkan','--use-webgpu-adapter=vulkan'],swiftshader:['--use-webgpu-adapter=swiftshader']};
const gpuFlags=GPU_FLAGS[option('gpu','')]||[];

const here=await mkdtemp(join(tmpdir(),'render2d-web-'));
const reference={};
let skipped=false;
if(!benchRun) {
  const native=option('native',resolve('build','win','render2dtest.exe'));
  const run=spawnSync(native,['--dump',here],{encoding:'utf8',env:{...process.env,jfgui_TEST_NOACTIVATE:'1'}});
  if(run.status!==0) {
    console.log(`FAIL the Vulkan path (${native}): ${run.status}\n${run.stdout}${run.stderr}`);
    process.exit(1);
  }
  console.log(run.stdout.trim().split('\n').pop());
  skipped=/SKIP/.test(run.stdout);
  for(const f of await readdir(here)) if(f.endsWith('.bgra')) reference[f.slice(0,-5)]=await readFile(join(here,f));
}

const pw=loadPlaywright(), dir=resolve(build,'web','render2dtest');
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
    const d=Math.max(Math.abs(a[i]-b[i]),Math.abs(a[i+1]-b[i+1]),Math.abs(a[i+2]-b[i+2]));
    if(d) {pixels++;largest=Math.max(largest,d);hist[Math.min(d,4)-1]++;}
  }
  return {pixels,largest,hist};
}

let failures=0, ran=0;
try {
  const browser=await pw.chromium.launch({headless:true,...chromiumOptions(),args:['--enable-unsafe-webgpu','--enable-webgpu-developer-features',...gpuFlags]});
  try {
    for(const mode of option('modes',benchRun?'simd':'simd,threads,scalar').split(',').filter(Boolean)) {
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
        if(benchRun&&t.startsWith('bench')) console.log(`${mode}: ${t}`);
        if(/^render2dtest: |^SKIP render2dtest/.test(t)) ok(t);
      }));
      page.on('pageerror',e=>lines.push(`pageerror ${e}`));
      let gpuenv=`jfgui_GPU_PROBE=${benchRun?'bench':'test'}`;
      if(benchRun&&option('n')) gpuenv+=`,jfgui_GPU_PROBE_N=${option('n')}`;
      if(benchRun&&args.includes('--thin')) gpuenv+=',jfgui_GPU_PROBE_THIN=1';
      if(benchRun&&args.includes('--plain')) gpuenv+=',jfgui_GPU_PROBE_PLAIN=1';
      if(benchRun&&args.includes('--g6a2b')) gpuenv+=',jfgui_GPU_PROBE_ARCS=1';
      await page.goto(`http://127.0.0.1:${server.address().port}/index.html?compute=${mode}&gpuenv=${gpuenv}`);
      const about=await page.evaluate(async()=>{try {const a=await navigator.gpu?.requestAdapter(); if(!a) return 'no adapter'; const i=a.info; return `${i.vendor} ${i.architecture} ${i.device} ${i.description} backend ${i.backendType} driver ${i.driver}`;} catch(e) {return String(e);}});
      console.log(`${mode}: the page's adapter: ${about}`);
      const end=await Promise.race([finished,new Promise(ok=>setTimeout(()=>ok('timeout'),benchRun?900000:150000).unref())]); // unref: the waiting limit must not keep node alive after the run
      const fail=lines.filter(l=>l.startsWith('FAIL')||l.startsWith('pageerror')||l.startsWith('WebGPU'));
      const count=lines.filter(l=>l.startsWith('ok ')).length;
      const compute=await page.evaluate(()=>window.__jfguiCompute?.mode);
      for(const l of fail) console.log(`${mode}: ${l}`);
      if(args.includes('--verbose')) for(const l of lines) console.log(l);
      if(end.startsWith('SKIP')) console.log(`SKIP ${mode}: ${end}`);
      else {
        ran++;
        let good=end.startsWith('render2dtest: ')&&/ 0 failed/.test(end)&&!fail.length&&count>0&&compute===mode;
        console.log(`${good?'PASS':'FAIL'} chromium ${browser.version()} ${compute}: ${end} (${lines.find(l=>l.startsWith('adapter:'))||'no adapter line'})`);
        for(const l of lines.filter(l=>/^ok .*(the drawing at|dash patterns at|1e-6 units|seed |culled|blend)/.test(l))) console.log(`  ${l}`);
        if(!benchRun&&!skipped) {
          let total=0;
          for(const level of ['voll','basis']) {
            const names=Object.keys(reference).filter(k=>k.startsWith(level+'_'));
            let worst=null, bad=0, differing=0, pixels=0;
            for(const key of names) {
              const name=key.slice(level.length+1);
              if(!images[name]) {
                console.log(`  FAIL no image ${name}`);
                good=false;
                continue;
              }
              const c=compare(images[name],reference[key]);
              const allowed=(images[name].length/4)*MAX_SHARE;
              const fine=c.largest<=MAX_DELTA||c.pixels<=allowed;
              pixels+=images[name].length/4;
              differing+=c.pixels;
              if(args.includes('--verbose')||!fine) console.log(`  ${fine?'ok  ':'FAIL'} ${name}: against Vulkan ${level}: ${c.pixels} pixels differ, largest ${c.largest}; by 1 / 2 / 3 / more: ${c.hist.join(' / ')}`);
              if(!fine) bad++;
              if(!worst||c.largest>worst.largest||c.largest===worst.largest&&c.pixels>worst.pixels) worst={...c,name};
            }
            console.log(`  ${bad?'FAIL':'ok  '} ${names.length} images against Vulkan ${level}: ${differing} of ${pixels} pixels differ; the worst: ${worst?.name}: ${worst?.pixels} pixels, largest ${worst?.largest}; by 1 / 2 / 3 / more: ${worst?.hist.join(' / ')}`);
            good=good&&!bad&&names.length>0;
            total+=names.length;
          }
          if(!total) good=false;
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
