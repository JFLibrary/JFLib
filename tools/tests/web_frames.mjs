// Real WASM/worker regression: lossless input batches and incremental composition.
// node tools/tests/web_frames.mjs build/wasm-responsive/after [--apps=gallery,...]
import assert from 'node:assert/strict';
import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {resolve} from 'node:path';
import vm from 'node:vm';
import {createHash} from 'node:crypto';
import {performance} from 'node:perf_hooks';

const root=resolve('.'),build=resolve(process.argv[2]||'build/wasm-responsive/after');
const artifact=resolve(build,'frame-tests');await mkdir(artifact,{recursive:true});
const results=[];
async function appWorker(app,width=1280,height=800,dpi=96){
  let now=1000,pixels=new Uint8Array(),size=[0,0],frames=0,transferred=0,captureMs=0;
  const errors=[],recycled=[];
  const sandbox={URL,TextDecoder,TextEncoder,Uint8Array,Uint8ClampedArray,Uint32Array,Int32Array,ArrayBuffer,DataView,WebAssembly,Date,console,
    navigator:{languages:['en-US'],language:'en-US',hardwareConcurrency:4},
    location:{href:`http://localhost/${app}/worker.js`},performance:{timeOrigin:0,now:()=>now},fetch:async url=>{
      const b=await readFile(resolve(build,'web',app,new URL(url,`http://localhost/${app}/`).pathname.split('/').at(-1)));
      return {ok:true,arrayBuffer:async()=>b.buffer.slice(b.byteOffset,b.byteOffset+b.byteLength),json:async()=>JSON.parse(b.toString())};
    },postMessage:m=>{
      if(m.type==='frame'){
        const start=performance.now();frames++;
        if(size[0]!==m.width||size[1]!==m.height){size=[m.width,m.height];pixels=new Uint8Array(m.width*m.height*4);}
        // the patches lie in one buffer at their offsets (older hosts: a buffer per patch)
        for(const p of m.patches||[{x:0,y:0,width:m.width,height:m.height,pixels:m.pixels}]){
          const source=m.buffer?new Uint8Array(m.buffer,p.offset,p.width*p.height*4):new Uint8Array(p.pixels);transferred+=source.length;
          for(let y=0;y<p.height;y++)pixels.set(source.subarray(y*p.width*4,(y+1)*p.width*4),((p.y+y)*m.width+p.x)*4);
          if(!m.buffer)recycled.push(p.pixels);
        }
        if(m.buffer)recycled.push(m.buffer);
        captureMs+=performance.now()-start;
      }
      if(m.type==='error'||m.type==='exit')errors.push(m);
    }};
  sandbox.self=sandbox;
  vm.runInNewContext(await readFile(resolve(build,'web',app,'worker.js'),'utf8'),sandbox);
  async function send(data){await sandbox.onmessage({data});if(recycled.length)await sandbox.onmessage({data:{type:'recycle',buffers:recycled.splice(0)}});assert.deepEqual(errors,[]);}
  await send({type:'start',url:`${app}.wasm`,width,height,dpi,args:['--theme','light'],files:[],capabilities:4});
  return {send,advance:ms=>{now+=ms;},hash:()=>createHash('sha256').update(pixels).digest('hex'),
    reset:()=>{frames=0;transferred=0;captureMs=0;},stats:()=>({frames,transferred,captureMs}),
    async fullCheck(){const before=this.hash();await send({type:'event',event:[8,0,0,0,...size,0]});assert.equal(this.hash(),before,'partial composition equals forced full composition');}};
}
const event=(type,x,y,button=0,data=0,mods=0)=>({type:'event',event:[type,button,mods,0,x,y,data>>>0]});
const only=process.argv.find(a=>a.startsWith('--apps='))?.slice(7).split(',');
for(const app of only||['template','gallery','Kritzel','PaintCraft','jfWord']){
  const one=await appWorker(app),batch=await appWorker(app);
  const initial=one.hash();assert.equal(batch.hash(),initial);
  // Curves, repeated pressure changes and release must not collapse to an endpoint.
  const steps=[event(1,380,350),event(2,380,350,1)];
  for(let i=1;i<=72;i++)steps.push(event(1,380+i*3,350+Math.round(Math.sin(i*.2)*65),0,(i%2?20000:55000)|(1<<16)));
  steps.push(event(3,596,350+Math.round(Math.sin(72*.2)*65),1));
  steps.push(event(5,0,0,0,17,2),event(5,0,0,0,90,2),event(6,0,0,0,90,2),event(6,0,0,0,17));
  // Redo keeps undo/redo edges in the same ordered input batch.
  steps.push(event(5,0,0,0,17,2),event(5,0,0,0,89,2),event(6,0,0,0,89,2),event(6,0,0,0,17));
  steps.push(event(8,1200,750),event(1,430,330),event(2,430,330,1),event(1,470,365),event(3,470,365,1),event(8,1280,800));
  for(const message of steps)await one.send(message);
  for(let i=0;i<steps.length;i+=19){await batch.send({type:'batch',messages:steps.slice(i,i+19)});await batch.fullCheck();}
  assert.equal(batch.hash(),one.hash(),`${app}: batched strokes/keys equal individually rendered input`);
  if(app==='Kritzel'||app==='PaintCraft')assert.notEqual(one.hash(),initial,'drawing changes the image');
  await one.fullCheck();
  // Menu overlay, close, resize and expose do not leave stale pixels behind.
  for(const message of [event(5,0,0,0,18,4),event(6,0,0,0,18),event(5,0,0,0,27),event(6,0,0,0,27),
    event(8,980,662),event(8,1280,800),event(1,620,400)]){
    await batch.send(message);await batch.fullCheck();
  }
  batch.reset();
  const idleStart=performance.now();for(let i=0;i<60;i++){batch.advance(33);await batch.send({type:'tick'});}
  const idle={...batch.stats(),ms:performance.now()-idleStart};
  batch.reset();const hoverStart=performance.now();
  for(let i=0;i<60;i++){batch.advance(33);await batch.send(event(1,380+(i%30)*8,400));}
  const hover={...batch.stats(),ms:performance.now()-hoverStart};
  hover.workerMs=hover.ms-hover.captureMs;
  const result={app,ok:true,strokeChanged:one.hash()!==initial,idle,hover};results.push(result);console.log(JSON.stringify(result));
}
// DPI and patch bounds at narrow/high-density viewports.
for(const app of ['PaintCraft','gallery'].filter(a=>!only||only.includes(a)))for(const dpi of [144,192]){
  const instance=await appWorker(app,950,1268,dpi);
  for(let i=0;i<12;i++){await instance.send(event(1,420+i*10,700));await instance.fullCheck();}
  results.push({app,dpi,ok:true});
}
await writeFile(resolve(artifact,'results.json'),JSON.stringify(results,null,2));
