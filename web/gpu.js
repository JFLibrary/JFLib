// lib/gpu's WebGPU backend, the browser's side (lib/gpu/web_*.rs, docs/GPU.md "Ausgabe im Browser").
// Loaded by worker.js with the first look at the GPU, in the worker that runs the app: WebGPU's objects
// belong to this thread, the compute workers of the threads bundle never see them.  WASM holds numbers
// (handles into objs); descriptors arrive as JSON, whose {"$":n} are handles; a submission's commands
// arrive as one run of words, recorded in lib/gpu/web_cmd.rs, and only here become a GPUCommandEncoder.
const METHODS=["createBuffer","createTexture","createSampler","createBindGroupLayout","createBindGroup","createComputePipeline","createRenderPipeline"];
const FEATURES=["depth32float-stencil8","rg11b10ufloat-renderable","bgra8unorm-storage","texture-compression-bc","float32-filterable","float32-blendable","indirect-first-instance","shader-f16","depth-clip-control","clip-distances","primitive-index","subgroups"];
const LIMITS=["maxTextureDimension1D","maxTextureDimension2D","maxTextureDimension3D","maxTextureArrayLayers","maxBindGroups","maxStorageBufferBindingSize","maxUniformBufferBindingSize","maxBufferSize","maxStorageBuffersPerShaderStage","maxUniformBuffersPerShaderStage","maxSampledTexturesPerShaderStage","maxSamplersPerShaderStage","maxStorageTexturesPerShaderStage","maxComputeWorkgroupStorageSize","maxComputeInvocationsPerWorkgroup","maxComputeWorkgroupSizeX","maxComputeWorkgroupSizeY","maxComputeWorkgroupSizeZ","maxComputeWorkgroupsPerDimension","maxColorAttachments","maxVertexBuffers","maxVertexAttributes","maxVertexBufferArrayStride"];
const PUSH_BYTES=128; // PUSH_CONSTANTS_MAX
const decode=new TextDecoder(), encode=new TextEncoder();
const u64=(lo,hi)=>lo+hi*4294967296;

export class GpuHost {
  constructor(memory,wake) {
    this.memory=memory;    // () => the WASM memory (a new buffer after it grew)
    this.wake=wake;        // the loop should look soon
    this.objs=[null];
    this.spare=[];
    this.start=null;       // the last start: state 1 on the way, 2 ready, 3 no adapter, 4 failed
    this.copyShared=false; // a browser that takes no shared memory in writeTexture
    this.fault=false;      // a call threw
  }
  add(o) {const h=this.spare.pop()||this.objs.length;this.objs[h]=o;return h;}
  bytes(p,n) {return new Uint8Array(this.memory().buffer,p,n);}
  text(p,n) {return decode.decode(this.bytes(p,n).slice());}
  answer(s,p,cap) {const b=encode.encode(s);this.bytes(p,cap).set(b.subarray(0,cap));return Math.min(b.length,cap);}
  // a descriptor: the JSON's {"$":n} are objects of the table
  desc(p,n) {return JSON.parse(this.text(p,n),(k,v)=>v&&typeof v==="object"&&"$" in v ? this.objs[v.$] : v);}
  source(data) {return this.copyShared&&data instanceof SharedArrayBuffer ? data.slice(0) : data;}

  begin() {
    this.fault=false;
    const s=this.start={state:1,adapter:null,device:null,error:""};
    (async()=>{
      const adapter=await navigator.gpu.requestAdapter({powerPreference:"low-power"});
      if(!adapter) {s.state=3;return;}
      const requiredLimits={};
      for(const k of LIMITS) if(adapter.limits[k]!==undefined) requiredLimits[k]=adapter.limits[k];
      const device=await adapter.requestDevice({requiredFeatures:FEATURES.filter(f=>adapter.features.has(f)),requiredLimits});
      s.adapter=adapter;s.device=device;s.state=2;
    })().catch(e=>{s.state=4;s.error=String(e?.message||e);}).finally(()=>this.wake());
  }
  // what the instance's adapter is: lines of name=value
  info() {
    const s=this.start, i=s.adapter.info||{}, d=s.device;
    const lines=[`vendor=${i.vendor||""}`,`arch=${i.architecture||""}`,`device=${i.device||""}`,`desc=${i.description||""}`,
      `fallback=${i.isFallbackAdapter||s.adapter.isFallbackAdapter?1:0}`,`format=${navigator.gpu.getPreferredCanvasFormat()}`,`features=${[...d.features].join(",")}`];
    for(const k of LIMITS) if(d.limits[k]!==undefined) lines.push(`${k}=${d.limits[k]}`);
    lines.push(`minUniformBufferOffsetAlignment=${d.limits.minUniformBufferOffsetAlignment}`,`minStorageBufferOffsetAlignment=${d.limits.minStorageBufferOffsetAlignment}`);
    if(d.limits.minSubgroupSize) lines.push(`minSubgroupSize=${d.limits.minSubgroupSize}`,`maxSubgroupSize=${d.limits.maxSubgroupSize}`);
    return lines.join("\n");
  }
  device() {
    const device=this.start?.device;
    if(!device) return 0;
    const d={device,queue:device.queue,lost:false,errors:0,message:"",done:0,last:0,watch:false};
    device.addEventListener("uncapturederror",e=>{
      const n=e.error?.constructor?.name;
      d.errors|=n==="GPUOutOfMemoryError"?4:n==="GPUInternalError"?8:2;
      d.message=String(e.error?.message||e.error);
      console.warn("WebGPU:",d.message);
      this.wake();
    });
    device.lost.then(i=>{d.lost=true;d.message=String(i?.message||"device lost");this.wake();});
    // push constants: a uniform buffer behind a dynamic offset, group 3 of every pipeline layout with them
    d.pushLayout=device.createBindGroupLayout({entries:[{binding:0,visibility:7,buffer:{type:"uniform",hasDynamicOffset:true}}]});
    d.emptyLayout=device.createBindGroupLayout({entries:[]});
    d.stride=Math.max(device.limits.minUniformBufferOffsetAlignment,PUSH_BYTES);
    this.pushRing(d,64);
    this.start=null;
    return this.add(d);
  }
  pushRing(d,slots) {
    const buffer=d.device.createBuffer({size:slots*d.stride,usage:GPUBufferUsage.UNIFORM|GPUBufferUsage.COPY_DST});
    d.ring={buffer,cpu:new Uint8Array(slots*d.stride),group:d.device.createBindGroup({layout:d.pushLayout,entries:[{binding:0,resource:{buffer,size:PUSH_BYTES}}]})};
  }
  // a Readback buffer the submission wrote comes to the CPU as soon as the GPU is done: Buffer::read
  fetch(b) {
    if(b.jf.busy) {b.jf.again=true;return;}
    b.jf.busy=true;b.jf.ready=false;
    b.mapAsync(GPUMapMode.READ).then(()=>{
      b.jf.data=new Uint8Array(b.getMappedRange()).slice();
      b.unmap();
    }).catch(()=>{b.jf.data=null;}).finally(()=>{
      b.jf.busy=false;b.jf.ready=true;
      if(b.jf.again) {b.jf.again=false;this.fetch(b);}
    });
  }

  // one submission: the words of web_cmd.rs (the OP_ numbers there)
  submit(d,p,n) {
    const w=new Uint32Array(this.memory().buffer,p,n), f=new Float32Array(this.memory().buffer,p,n), o=this.objs;
    const enc=d.device.createCommandEncoder();
    let ring=d.ring, pass=null, computing=false, slot=0, compute=null;
    const groups=[], reads=new Set(), push=new Uint8Array(PUSH_BYTES);
    let pushed=false, dirty=false;
    const endPass=()=>{if(pass) {pass.end();pass=null;computing=false;}};
    const place=()=>{
      if(!dirty) return;
      dirty=false;
      // the ring is full: its values go up (before the submission, as the last ring's below), a larger one takes over
      if((slot+1)*d.stride>ring.cpu.length) {
        d.queue.writeBuffer(ring.buffer,0,ring.cpu,0,slot*d.stride);
        this.pushRing(d,ring.cpu.length/d.stride*4);
        ring=d.ring;slot=0;
      }
      ring.cpu.set(push,slot*d.stride);
      pass.setBindGroup(3,ring.group,[slot*d.stride]);
      slot++;
    };
    const dispatching=()=>{
      if(!computing) {
        endPass();
        pass=enc.beginComputePass();computing=true;
        if(compute) pass.setPipeline(compute);
        groups.forEach((g,i)=>{if(g) pass.setBindGroup(i,g);});
        dirty=pushed;
      }
      place();
    };
    for(let i=0;i<n;) {
      const op=w[i++];
      switch(op) {
        case 0: endPass();compute=null;groups.length=0;pushed=false;dirty=false;break; // the next command buffer
        case 1: endPass();enc.copyBufferToBuffer(o[w[i]],u64(w[i+1],w[i+2]),o[w[i+3]],u64(w[i+4],w[i+5]),u64(w[i+6],w[i+7]));if(o[w[i+3]].jf)reads.add(o[w[i+3]]);i+=8;break;
        case 2: { // fill: zero is clearBuffer, anything else comes from a small buffer of its own
          endPass();
          const b=o[w[i]],off=u64(w[i+1],w[i+2]),size=u64(w[i+3],w[i+4]),v=w[i+5];i+=6;
          if(!v) enc.clearBuffer(b,off,size);
          else {
            const s=d.device.createBuffer({size,usage:GPUBufferUsage.COPY_SRC,mappedAtCreation:true});
            new Uint32Array(s.getMappedRange()).fill(v);s.unmap();
            enc.copyBufferToBuffer(s,0,b,off,size);
          }
          break;
        }
        case 3: endPass();enc.copyBufferToTexture({buffer:o[w[i]],offset:u64(w[i+1],w[i+2]),bytesPerRow:w[i+3],rowsPerImage:w[i+4]},{texture:o[w[i+5]],mipLevel:w[i+6],origin:[w[i+7],w[i+8],w[i+9]]},[w[i+10],w[i+11],w[i+12]]);i+=13;break;
        case 4: endPass();enc.copyTextureToBuffer({texture:o[w[i]],mipLevel:w[i+1],origin:[w[i+2],w[i+3],w[i+4]]},{buffer:o[w[i+8]],offset:u64(w[i+9],w[i+10]),bytesPerRow:w[i+11],rowsPerImage:w[i+12]},[w[i+5],w[i+6],w[i+7]]);if(o[w[i+8]].jf)reads.add(o[w[i+8]]);i+=13;break;
        case 5: compute=o[w[i++]];if(computing) pass.setPipeline(compute);break;
        case 6: { // a group: of the render pass, or for the compute passes to come
          const index=w[i++],g=o[w[i++]];
          if(pass) pass.setBindGroup(index,g);
          if(!pass||computing) groups[index]=g;
          break;
        }
        case 7: { // push constants: offset, words
          const off=w[i++],words=w[i++];
          push.set(new Uint8Array(this.memory().buffer,p+i*4,words*4),off);
          i+=words;pushed=true;dirty=true;
          break;
        }
        case 8: dispatching();pass.dispatchWorkgroups(w[i],w[i+1],w[i+2]);i+=3;break;
        case 9: dispatching();pass.dispatchWorkgroupsIndirect(o[w[i]],u64(w[i+1],w[i+2]));i+=3;break;
        case 10: { // a render pass: colours (view, load, store, r, g, b, a), a depth attachment (view 0: none)
          endPass();
          const colors=w[i++], colorAttachments=[];
          for(let k=0;k<colors;k++,i+=7) colorAttachments.push({view:o[w[i]],loadOp:w[i+1]?"load":"clear",storeOp:w[i+2]?"store":"discard",clearValue:[f[i+3],f[i+4],f[i+5],f[i+6]]});
          const desc={colorAttachments};
          if(w[i]) {
            const a={view:o[w[i]],depthLoadOp:w[i+1]?"load":"clear",depthClearValue:f[i+3],depthStoreOp:w[i+2]?"store":"discard"};
            if(w[i+4]) {a.stencilLoadOp="clear";a.stencilStoreOp="discard";}
            desc.depthStencilAttachment=a;
          }
          i+=5;
          pass=enc.beginRenderPass(desc);dirty=pushed;
          break;
        }
        case 11: pass.end();pass=null;break;
        case 12: pass.setPipeline(o[w[i++]]);break;
        case 13: pass.setViewport(f[i],f[i+1],f[i+2],f[i+3],f[i+4],f[i+5]);i+=6;break;
        case 14: pass.setScissorRect(w[i],w[i+1],w[i+2],w[i+3]);i+=4;break;
        case 15: pass.setVertexBuffer(w[i],o[w[i+1]],u64(w[i+2],w[i+3]));i+=4;break;
        case 16: pass.setIndexBuffer(o[w[i]],w[i+3]?"uint32":"uint16",u64(w[i+1],w[i+2]));i+=4;break;
        case 17: place();pass.draw(w[i],w[i+1],w[i+2],w[i+3]);i+=4;break;
        case 18: place();pass.drawIndexed(w[i],w[i+1],w[i+2],w[i+3]|0,w[i+4]);i+=5;break;
        case 19: place();pass.drawIndirect(o[w[i]],u64(w[i+1],w[i+2]));i+=3;break;
        case 20: place();pass.drawIndexedIndirect(o[w[i]],u64(w[i+1],w[i+2]));i+=3;break;
        default: throw new Error("gpu: unknown command "+op);
      }
    }
    endPass();
    if(slot) d.queue.writeBuffer(ring.buffer,0,ring.cpu,0,slot*d.stride);
    d.queue.submit([enc.finish()]);
    for(const b of reads) this.fetch(b);
    return ++d.last;
  }
  writeRects(d,tex,rects,count,data,len,stride,ox,oy) {
    const r=new Int32Array(this.memory().buffer,rects,count*4), source=this.source(this.memory().buffer), t=this.objs[tex];
    const layout={offset:0,bytesPerRow:stride}, size=[0,0], at={texture:t,origin:[0,0]};
    for(let k=0;k<count;k++) {
      const x=r[k*4],y=r[k*4+1];
      layout.offset=data+y*stride+x*4;at.origin[0]=x-ox;at.origin[1]=y-oy;size[0]=r[k*4+2];size[1]=r[k*4+3];
      try {d.queue.writeTexture(at,source,layout,size);}
      catch(error) {
        // (a browser without AllowSharedBufferSource here: the bytes of a copy from now on)
        if(this.copyShared||!(error instanceof TypeError)) throw error;
        this.copyShared=true;
        return this.writeRects(d,tex,rects,count,data,len,stride,ox,oy);
      }
    }
  }

  // a call from WASM.  Nothing may throw through it (the app's worker would stop): what the browser
  // refuses here counts as an internal error of the device, the next look goes back to the old path
  call(op,args,count) {
    try {return this.dispatch(op,args,count);}
    catch(error) {
      console.warn("WebGPU:",error);
      this.fault=true;
      return 0;
    }
  }
  dispatch(op,args,count) {
    const a=new Uint32Array(this.memory().buffer,args,count), o=this.objs;
    switch(op) {
      case 0:
        if(a[0]) this.begin();
        return this.start?.state||1;
      case 1: return this.answer(this.start?.adapter&&this.start.device?this.info():"",a[0],a[1]);
      case 2: return this.device();
      case 3: { // free: buffers and textures give their memory back (what is in flight finishes first)
        const x=o[a[0]];
        if(a[1]) x?.destroy?.();
        // (a canvas's images: the page takes the canvas away after the frame that replaces it, not before)
        if(x?.context) setTimeout(()=>{try{x.context.unconfigure();}catch{}},1000);
        o[a[0]]=null;this.spare.push(a[0]);
        return 0;
      }
      case 4: { // a new object from a descriptor: of the device, or a view of a texture
        const p=o[a[0]], desc=this.desc(a[2],a[3]);
        if(a[1]===100) return this.add(p.createView(desc));
        const x=p.device[METHODS[a[1]]](desc);
        if(a[1]===0&&desc.usage&GPUBufferUsage.MAP_READ) x.jf={busy:false,ready:false,again:false,data:null};
        return this.add(x);
      }
      case 5: return this.add(o[a[0]].device.createShaderModule({code:this.text(a[1],a[2])}));
      case 6: o[a[0]].queue.writeBuffer(o[a[1]],u64(a[2],a[3]),this.source(this.memory().buffer),a[4],a[5]);return 0;
      case 7: { // Buffer::read: 0 not here yet (the GPU is not done)
        const b=o[a[0]], off=u64(a[1],a[2]);
        if(b.jf.busy||b.jf.again) return 0;
        const out=this.bytes(a[3],a[4]);
        if(b.jf.data) out.set(b.jf.data.subarray(off,off+a[4]));
        else out.fill(0);
        return 1;
      }
      case 8: o[a[0]].queue.writeTexture({texture:o[a[1]],mipLevel:a[2],origin:[a[3],a[4],a[5]]},this.source(this.memory().buffer),{offset:a[9],bytesPerRow:a[11],rowsPerImage:a[12]},[a[6],a[7],a[8]]);return 0;
      case 9: this.writeRects(o[a[0]],a[1],a[2],a[3],a[4],a[5],a[6],a[7]|0,a[8]|0);return 0;
      case 10: { // a canvas's context: configured for this device and size
        const canvas=o[a[1]];
        canvas.width=a[2];canvas.height=a[3];
        const context=canvas.getContext("webgpu");
        if(!context) return 0;
        const format=navigator.gpu.getPreferredCanvasFormat();
        const c={context,canvas,device:o[a[0]].device,format,usage:GPUTextureUsage.RENDER_ATTACHMENT|(a[4]?GPUTextureUsage.COPY_SRC:0),srgb:a[5]?format+"-srgb":null};
        context.configure({device:c.device,format,usage:c.usage,alphaMode:"opaque",colorSpace:"srgb",viewFormats:c.srgb?[c.srgb]:[]});
        return this.add(c);
      }
      case 11: { // the next image: its texture and view take these two handles
        const c=o[a[0]], t=c.context.getCurrentTexture();
        o[a[1]]=t;o[a[2]]=c.srgb?t.createView({format:c.srgb}):t.createView();
        return 1;
      }
      case 12: { // a new size
        const c=o[a[0]];
        c.canvas.width=a[1];c.canvas.height=a[2];
        return 0;
      }
      case 13: return this.submit(o[a[0]],a[1],a[2]);
      case 14: {const d=o[a[0]];return (d.lost?1:0)|d.errors|(this.fault?8:0);}
      case 16: { // is this submission done?  One watcher at a time, for the newest submission
        const d=o[a[0]];
        if(d.done>=a[1]) return 1;
        if(!d.watch) {
          d.watch=true;
          const upto=d.last;
          d.queue.onSubmittedWorkDone().then(()=>{d.done=Math.max(d.done,upto);},()=>{d.done=Math.max(d.done,upto);}).finally(()=>{d.watch=false;this.wake();});
        }
        return 0;
      }
      case 17: { // (a test aid) lose it
        const d=o[a[0]];
        d.device.destroy();d.lost=true;
        return 0;
      }
      case 18: return this.add(null); // a handle for what comes later
      case 19: { // a pipeline layout: the groups (0: none), with push constants the ring's group 3
        const d=o[a[0]], push=a[1], n=a[2], list=[];
        for(let k=0;k<n;k++) list.push(a[3+k]?o[a[3+k]]:d.emptyLayout);
        if(push) {while(list.length<3) list.push(d.emptyLayout);list.push(d.pushLayout);}
        return this.add(d.device.createPipelineLayout({bindGroupLayouts:list}));
      }
      case 20: return navigator.gpu.getPreferredCanvasFormat()==="rgba8unorm"?1:0;
      default: throw new Error("gpu: unknown call "+op);
    }
  }
  // (worker.js) a canvas from the page
  canvas(offscreen) {return this.add(offscreen);}
}
