// Safari for the browser tests (docs/APPLE.md AP1): the part of Playwright's API that
// tools/tests/web_apps.mjs uses, over W3C WebDriver to safaridriver.  Playwright drives no real
// Safari (its "webkit" is a WebKit build of its own), so this talks to /usr/bin/safaridriver on a
// Mac -- `safaridriver --enable` once, then Safari's Develop menu: "Allow Remote Automation".
//
//   SAFARIDRIVER_URL   the driver (default http://127.0.0.1:4719): on the Mac itself
//                      `safaridriver --port 4719`; from another machine through an SSH tunnel
//                      (tools/tests/safari_tunnel.py does both directions)
//
// What WebDriver cannot do, the adapter does in the page or leaves out, and says so:
// - addInitScript runs right after each navigation (not before the page's own scripts);
// - console errors and uncaught errors are collected by a hook installed after navigation
//   (errors before that are not seen);
// - setInputFiles builds the File in the page and hands it to the input through DataTransfer
//   (a synthetic change event, no file dialog);
// - a context's locale and device scale factor are Safari's own (the Mac's language, the
//   display's scale): `emulates` says which options were ignored.
import {writeFile} from 'node:fs/promises';

const ELEMENT='element-6066-11e4-a52e-4f735466cecf';
const KEYS={Escape:'',Enter:'',Tab:'',Backspace:'',ArrowLeft:'',ArrowUp:'',ArrowRight:'',ArrowDown:''};
const ERROR_HOOK=`if(!window.__wdErrors){window.__wdErrors=[];
  addEventListener('error',e=>window.__wdErrors.push(String(e.message||e)));
  addEventListener('unhandledrejection',e=>window.__wdErrors.push('unhandled rejection: '+String(e.reason&&e.reason.message||e.reason)));
  const ce=console.error.bind(console);console.error=(...a)=>{window.__wdErrors.push(a.map(String).join(' '));ce(...a);};}`;

async function call(base,method,path,body) {
  const response=await fetch(base+path,{method,headers:{'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});
  const json=await response.json().catch(()=>({}));
  if(!response.ok || json.value?.error) throw new Error(`WebDriver ${method} ${path}: ${json.value?.error||response.status} ${json.value?.message||''}`.trim());
  return json.value;
}

// a function and its argument for executeAsync: the result or the error comes back as {ok}/{err}
function script(fn,withElement) {
  return `const done=arguments[arguments.length-1];
    try{Promise.resolve((${fn})(${withElement?'arguments[0],arguments[1]':'arguments[0]'})).then(v=>done({ok:v===undefined?null:v}),e=>done({err:String(e&&e.stack||e)}));}
    catch(e){done({err:String(e&&e.stack||e)});}`;
}

class Page {
  constructor(context) {this.context=context;this.init=[];this.handlers={pageerror:[],console:[]};this.keyboard=new Keyboard(this);this.mouse=new Mouse(this);}
  get s() {return this.context.session;}
  on(type,callback) {(this.handlers[type]||=[]).push(callback);}
  async addInitScript(fn,arg) {this.init.push([fn,arg]);}
  async goto(url) {
    await this.s('POST','/url',{url});
    await this.s('POST','/execute/sync',{script:ERROR_HOOK,args:[]});
    for(const [fn,arg] of this.init) await this.evaluate(fn,arg);
  }
  async errors() {
    const list=await this.s('POST','/execute/sync',{script:'const e=window.__wdErrors||[];window.__wdErrors=[];return e;',args:[]}).catch(()=>[]);
    for(const text of list) for(const h of this.handlers.pageerror) h(text);
  }
  async evaluate(fn,arg) {
    const r=await this.s('POST','/execute/async',{script:script(fn,false),args:[arg===undefined?null:arg]});
    await this.errors();
    if(r && 'err' in r) throw new Error(`in Safari: ${r.err}`);
    return r?.ok;
  }
  async waitForFunction(fn,{timeout=30000}={}) {
    const end=Date.now()+timeout;
    for(;;) {
      if(await this.evaluate(fn).catch(()=>false)) return true;
      if(Date.now()>end) throw new Error(`waitForFunction timed out after ${timeout} ms: ${fn}`);
      await new Promise(r=>setTimeout(r,100));
    }
  }
  locator(selector) {return new Locator(this,selector);}
  async setViewportSize({width,height}) {
    const [dx,dy]=await this.evaluate(()=>[outerWidth-innerWidth,outerHeight-innerHeight]);
    await this.s('POST','/window/rect',{width:width+dx,height:height+dy});
    const got=await this.evaluate(()=>[innerWidth,innerHeight]);
    if(got[0]!==width||got[1]!==height) this.context.emulates.viewport=`asked ${width}x${height}, got ${got[0]}x${got[1]} (the screen)`;
  }
  async screenshot({path}={}) {
    const png=Buffer.from(await this.s('GET','/screenshot'),'base64');
    if(path) await writeFile(path,png);
    return png;
  }
  async element(selector) {return (await this.s('POST','/element',{using:'css selector',value:selector}))[ELEMENT];}
}

class Locator {
  constructor(page,selector) {this.page=page;this.selector=selector;}
  async focus() {await this.page.evaluate(sel=>{document.querySelector(sel).focus();},this.selector);}
  async click() {const id=await this.page.element(this.selector);await this.page.s('POST',`/element/${id}/click`,{});await this.page.errors();}
  async evaluate(fn,arg) {
    const r=await this.page.s('POST','/execute/async',{script:`const el=document.querySelector(${JSON.stringify(this.selector)});`+script(fn,true).replace('arguments[0],arguments[1]','el,arguments[0]'),args:[arg===undefined?null:arg]});
    await this.page.errors();
    if(r && 'err' in r) throw new Error(`in Safari: ${r.err}`);
    return r?.ok;
  }
  async boundingBox() {return this.page.evaluate(sel=>{const b=document.querySelector(sel).getBoundingClientRect();return {x:b.x,y:b.y,width:b.width,height:b.height};},this.selector);}
  async textContent() {return this.page.evaluate(sel=>document.querySelector(sel)?.textContent??null,this.selector);}
  async setInputFiles({name,mimeType,buffer}) {
    await this.page.evaluate(({sel,name,type,data})=>{
      const bytes=Uint8Array.from(atob(data),c=>c.charCodeAt(0));
      const t=new DataTransfer();t.items.add(new File([bytes],name,{type}));
      const input=document.querySelector(sel);input.files=t.files;
      input.dispatchEvent(new Event('change',{bubbles:true}));
    },{sel:this.selector,name,type:mimeType,data:Buffer.from(buffer).toString('base64')});
  }
}

class Keyboard {
  constructor(page) {this.page=page;}
  async insertText(text) {
    // the focused element gets the text as typed characters: WebDriver has no "insert" without keys,
    // and its element input refuses the host's hidden text field, so key actions go to the focus
    const actions=[];
    for(const ch of text) actions.push({type:'keyDown',value:ch},{type:'keyUp',value:ch});
    await this.page.s('POST','/actions',{actions:[{type:'key',id:'keyboard',actions}]});
    await this.page.s('DELETE','/actions');
  }
  async press(key) {
    const k=KEYS[key]||key;
    await this.page.s('POST','/actions',{actions:[{type:'key',id:'keyboard',actions:[{type:'keyDown',value:k},{type:'keyUp',value:k}]}]});
    await this.page.s('DELETE','/actions');
  }
}

class Mouse {
  constructor(page) {this.page=page;}
  async click(x,y) {
    await this.page.s('POST','/actions',{actions:[{type:'pointer',id:'mouse',parameters:{pointerType:'mouse'},actions:[
      {type:'pointerMove',duration:0,origin:'viewport',x:Math.round(x),y:Math.round(y)},
      {type:'pointerDown',button:0},{type:'pointerUp',button:0}]}]});
    await this.page.s('DELETE','/actions');
  }
}

class Context {
  constructor(base,options) {this.base=base;this.options=options;this.emulates={};this.id=null;}
  async start() {
    const v=await call(this.base,'POST','/session',{capabilities:{alwaysMatch:{browserName:'safari'}}});
    this.id=v.sessionId;
    this.version=v.capabilities?.browserVersion;
    this.session=(method,path,body)=>call(this.base,method,`/session/${this.id}${path}`,body);
    if(this.options.locale) this.emulates.locale=`asked ${this.options.locale}, Safari keeps the Mac's language`;
    if(this.options.deviceScaleFactor && this.options.deviceScaleFactor!==1) this.emulates.deviceScaleFactor=`asked ${this.options.deviceScaleFactor}, Safari keeps the display's`;
    return this;
  }
  async newPage() {
    const page=new Page(this);
    await this.session('POST','/url',{url:'about:blank'});
    if(this.options.viewport) await page.setViewportSize(this.options.viewport);
    return page;
  }
  async close() {if(this.id) await call(this.base,'DELETE',`/session/${this.id}`).catch(()=>{});this.id=null;}
}

// launch(): Safari behind SAFARIDRIVER_URL; one session (context) at a time, as Safari allows
export async function launchSafari() {
  const base=(process.env.SAFARIDRIVER_URL||'http://127.0.0.1:4719').replace(/\/$/,'');
  const status=await call(base,'GET','/status').catch(e=>{throw new Error(`no safaridriver at ${base} (${e.message})`);});
  return {
    status,
    async newContext(options={}) {return new Context(base,options).start();},
    async close() {},
  };
}
