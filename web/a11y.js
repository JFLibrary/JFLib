// The accessibility tree of a jfgui app as an ARIA shadow tree in the DOM. The app (WASM in
// the worker) sends the differences of its tree (lib/plat/a11y_wire.rs: the format) as
// service requests of kind 11; Mirror keeps the tree, Shadow keeps one transparent element
// per node over the canvas at the node's rectangle: roles, names, states, values, real
// text fields with the app's text and selection, a range input per slider. The DOM focus
// follows the app's focus once the keyboard or assistive technology drives it (a pointer on
// the canvas gives it back to the hidden #input); clicks, focus, value and text edits and
// selections of assistive technology go back as actions (worker: jfgui_a11y_act).
// Mirror and decode need no DOM (node tests: apps/androidGallery/tests/a11y.mjs).

export const VERSION=1;
/** Node::role in plat::a11y::Role declaration order */
export const ROLES=["group","label","button","checkbox","radio","switch","slider","progress","textbox","link",
  "list","listitem","menubar","menu","menuitem","tablist","tab","combo","image","canvas","dialog"];
/** Node::state bits (plat::a11y::S_*) */
export const S={FOCUSED:1,FOCUSABLE:2,DISABLED:4,CHECKED:8,MIXED:16,SELECTED:32,EXPANDED:64,PRESSED:128,
  MULTILINE:256,PROTECTED:512,READONLY:1024};
/** Node::actions bits (plat::a11y::A_*) */
export const A={CLICK:1,FOCUS:2,VALUE:4,TEXT:8};
/** jfgui_a11y_act kinds */
export const ACT={CLICK:1,FOCUS:2,VALUE:3,TEXT:4,SELECT:5};
/** update events (a11y_wire.rs E_*) */
export const EV={WINDOW:1,ACTIVE:2,FOCUS:3,STATE:4,NAME:5,VALUE:6,TEXT:7,CARET:8};
const NONE=0xFFFFFFFF, utf8=new TextDecoder();

/** One update (ArrayBuffer or typed array) -> {full, wins:[...], events:[...]}; keys are
 * {lo, hi, id} with id a string unique within the window. */
export function decode(data) {
  const view=ArrayBuffer.isView(data) ? new DataView(data.buffer,data.byteOffset,data.byteLength) : new DataView(data);
  let p=0;
  const u8=()=>view.getUint8(p++);
  const u32=()=>{const v=view.getUint32(p,true);p+=4;return v;};
  const i32=()=>{const v=view.getInt32(p,true);p+=4;return v;};
  const f64=()=>{const v=view.getFloat64(p,true);p+=8;return v;};
  const key=()=>{const lo=u32(),hi=u32();return {lo,hi,id:hi.toString(16)+"."+lo.toString(16)};};
  const str=()=>{const n=u32();if(p+n>view.byteLength)throw new RangeError("a11y: string beyond the update");
    const s=utf8.decode(new Uint8Array(view.buffer,view.byteOffset+p,n));p+=n;return s;};
  const rect=()=>[i32(),i32(),i32(),i32()];
  if(u8()!==VERSION) throw new Error("a11y: unknown update version");
  const full=(u8()&1)!==0, wins=[];
  for(let w=u8();w>0;w--) {
    const win=u8(), flags=u8(), place=rect(), focus=key(), title=str();
    let order=null;
    const count=u32();
    if(count!==NONE) {order=[];for(let i=0;i<count;i++){const k=key();order.push({key:k,parent:u32()});}}
    const nodes=[];
    for(let n=u32();n>0;n--) nodes.push({key:key(),role:u8(),state:u32(),actions:u32(),rect:rect(),name:str(),text:str(),
      caret:i32(),anchor:i32(),caretRect:rect(),value:f64(),min:f64(),max:f64(),step:f64()});
    wins.push({win,dialog:(flags&1)!==0,active:(flags&2)!==0,place,focus,title,order,nodes});
  }
  const events=[];
  for(let n=u32();n>0;n--) {
    const kind=u8(),win=u8(),k=key(),a=i32(),b=i32();
    events.push({kind,win,key:k,a,b,old:kind===EV.TEXT?str():undefined});
  }
  if(p!==view.byteLength) throw new Error("a11y: bytes after the update");
  return {full,wins,events};
}

/** The tree as the host has it: wins (Map window index -> {win, dialog, active, place,
 * focus (node id or null), title, order (node ids, parents first), parent (Map id -> id or
 * null), nodes (Map id -> node)}) in drawing order. */
export class Mirror {
  constructor() {this.wins=new Map();}
  /** apply a decoded update -> {structure: windows whose nodes came or went, changed: nodes
   * (win, id) that are new or changed} */
  apply(u) {
    if(u.full) this.wins.clear();
    const next=new Map(), structure=new Set(), changed=[];
    for(const w of u.wins) {
      let t=this.wins.get(w.win);
      if(!t) {t={win:w.win,order:[],parent:new Map(),nodes:new Map()};structure.add(w.win);}
      if(!w.order && !this.wins.has(w.win)) throw new Error("a11y: a new window without its order");
      Object.assign(t,{dialog:w.dialog,active:w.active,place:w.place,focus:w.focus.lo||w.focus.hi?w.focus.id:null,title:w.title});
      if(w.order) {
        t.order=w.order.map(e=>e.key.id);
        t.parent=new Map(w.order.map((e,i)=>[t.order[i],e.parent===NONE?null:t.order[e.parent]]));
        for(const id of t.nodes.keys()) if(!t.parent.has(id)) t.nodes.delete(id);
        structure.add(w.win);
      }
      for(const n of w.nodes) {
        n.id=n.key.id; n.win=w.win;
        if(!t.parent.has(n.id)) throw new Error("a11y: a node outside the order");
        t.nodes.set(n.id,n); changed.push(n);
      }
      next.set(w.win,t);
    }
    for(const win of this.wins.keys()) if(!next.has(win)) structure.add(win);
    this.wins=next;
    return {structure,changed};
  }
  /** the window that has the keyboard */
  active() {for(const w of this.wins.values()) if(w.active) return w; return null;}
  /** the nodes of window win whose name or text contains s (tests) */
  find(win,s) {return [...(this.wins.get(win)?.nodes.values() || [])].filter(n=>n.name.includes(s) || n.text.includes(s));}
}

// characters (Unicode scalar values, the app's unit) <-> UTF-16 code units (the DOM's)
export function charsToUnits(text,chars) {
  let i=0;
  for(let c=0;c<chars && i<text.length;c++) i+=text.codePointAt(i)>0xFFFF?2:1;
  return i;
}
export function unitsToChars(text,units) {
  let c=0;
  for(let i=0;i<units && i<text.length;c++) i+=text.codePointAt(i)>0xFFFF?2:1;
  return c;
}

// roles whose element holds other elements; the children of all others (a popup below the
// drop-down or menu entry that opened it) go to the nearest holder, the opener points at them
const HOLDERS=new Set(["group","list","menubar","tablist","dialog"]);
const isSyn=n=>(n.key.hi&0x80000000)!==0;
const holds=n=>HOLDERS.has(ROLES[n.role]) || (ROLES[n.role]==="menu" && isSyn(n));

function ariaRole(n) {
  switch(ROLES[n.role]) {
    case "group": return "group";
    case "button": return "button";
    case "checkbox": return "checkbox";
    case "radio": return "radio";
    case "switch": return "switch";
    case "progress": return "progressbar";
    case "link": return "link";
    case "list": return "listbox";
    case "listitem": return "option";
    case "menubar": return "menubar";
    case "menu": return isSyn(n) ? "menu" : "menuitem";
    case "menuitem": return n.state&S.CHECKED ? "menuitemcheckbox" : "menuitem";
    case "tablist": return "tablist";
    case "tab": return "tab";
    case "combo": return "combobox";
    case "image": case "canvas": return "img";
    case "dialog": return "dialog";
    default: return null; // labels: their text; text boxes and sliders: native elements
  }
}
// the element a node needs (a node whose kind changes gets a new one)
function kindOf(n) {
  const r=ROLES[n.role];
  if(r==="textbox") return n.state&S.MULTILINE ? "textarea" : n.state&S.PROTECTED ? "password" : "text";
  if(r==="slider") return "range";
  return "div";
}
const isField=el=>el.tagName==="TEXTAREA" || (el.tagName==="INPUT" && el.type!=="range");
const set=(el,name,value)=>{if(value===null || value===undefined) {if(el.hasAttribute(name)) el.removeAttribute(name);} else if(el.getAttribute(name)!==String(value)) el.setAttribute(name,String(value));};

/** The ARIA shadow tree in root (a positioned element over the canvas, transparent and
 * without pointer events). host: act(message) sends an action {win, lo, hi, act, a, b, value,
 * text}; input: the hidden text area the keyboard goes to while a pointer drives the app. */
export class Shadow {
  constructor(root,{act,input}) {
    this.root=root; this.send=act; this.input=input; this.mirror=new Mirror();
    this.wins=new Map();          // win -> {el, els: Map id -> element}
    this.follow=false;            // the DOM focus follows the app's focus
    this.own=false;               // a focus() of ours is under way
    this.composing=null;          // the text field with an IME composition
    this.synced=new WeakMap();    // text field -> [start, end, direction] set last
    this.pending=new WeakMap();   // text field -> {text, until}: an edit the app has not shown yet
    this.tabbed=null;             // the element with tabindex 0
    this.updates=0;               // updates applied (tests wait for them)
    root.addEventListener("click",e=>this.clicked(e));
    root.addEventListener("focusin",e=>this.focused(e));
    root.addEventListener("compositionstart",e=>{this.composing=e.target;});
    root.addEventListener("compositionend",()=>{this.composing=null;});
    document.addEventListener("selectionchange",()=>this.selected());
    root.addEventListener("scroll",()=>{root.scrollTop=0;root.scrollLeft=0;});
  }
  /** apply a decoded update */
  apply(u) {
    this.updates++;
    this.claimed=null;
    const had=this.root.contains(document.activeElement);
    const {structure,changed}=this.mirror.apply(u);
    for(const [win,v] of this.wins) if(!this.mirror.wins.has(win)) {v.el.remove();this.wins.delete(win);}
    for(const t of this.mirror.wins.values()) {
      let v=this.wins.get(t.win);
      if(!v) {v={el:document.createElement("div"),els:new Map()};v.el.dataset.win=t.win;this.wins.set(t.win,v);}
      this.window(t,v);
    }
    // the windows in drawing order
    let prev=null;
    for(const t of this.mirror.wins.values()) {
      const el=this.wins.get(t.win).el;
      if(el.parentNode!==this.root || el.previousElementSibling!==prev) this.root.insertBefore(el,prev?prev.nextSibling:this.root.firstChild);
      prev=el;
    }
    for(const n of changed) this.node(this.mirror.wins.get(n.win),n);
    for(const win of structure) if(this.mirror.wins.has(win)) this.arrange(this.mirror.wins.get(win));
    if(structure.size || changed.length) this.place();
    this.focus(had && !this.root.contains(document.activeElement));
  }
  // the window's element
  window(t,v) {
    const el=v.el;
    set(el,"role",t.win===0 ? "region" : "dialog");
    set(el,"aria-label",t.title || null);
    set(el,"aria-modal",t.dialog ? "true" : null);
  }
  // the element of node n (new, or replaced when it needs another kind of element)
  node(t,n) {
    const v=this.wins.get(t.win), kind=kindOf(n);
    let el=v.els.get(n.id);
    if(el && el.dataset.kind!==kind) {const old=el;el=null;old.replaceWith(this.make(t,n,kind,v));el=v.els.get(n.id);}
    if(!el) el=this.make(t,n,kind,v);
    const r=ROLES[n.role], st=n.state;
    set(el,"role",ariaRole(n));
    if(r==="label") {if(el.textContent!==n.name) el.textContent=n.name; set(el,"aria-label",null);}
    else if(r==="combo") {if(el.textContent!==n.text) el.textContent=n.text; set(el,"aria-label",n.name || null);}
    else set(el,"aria-label",n.name || null);
    set(el,"aria-disabled",st&S.DISABLED ? "true" : null);
    set(el,"tabindex",st&S.FOCUSABLE ? (el===this.tabbed ? "0" : "-1") : null);
    const checks=["checkbox","radio","switch"].includes(r) || (r==="menuitem" && st&S.CHECKED);
    set(el,"aria-checked",checks ? (st&S.MIXED ? "mixed" : st&S.CHECKED ? "true" : "false") : null);
    set(el,"aria-pressed",r==="button" && (st&S.PRESSED || el.hasAttribute("aria-pressed")) ? String(!!(st&S.PRESSED)) : null);
    set(el,"aria-selected",r==="tab" || r==="listitem" ? String(!!(st&S.SELECTED)) : null);
    const opener=r==="combo" || (r==="menu" && !isSyn(n));
    set(el,"aria-haspopup",r==="combo" ? "listbox" : opener ? "menu" : null);
    set(el,"aria-expanded",opener ? String(!!(st&S.EXPANDED)) : !holds(n) && st&S.EXPANDED ? "true" : null);
    if(r==="progress") {
      set(el,"aria-valuemin",n.min); set(el,"aria-valuemax",n.max);
      set(el,"aria-valuenow",n.value<n.min ? null : n.value);
    }
    if(kind==="range") {
      for(const [k,x] of [["min",n.min],["max",n.max],["step",n.step || 1]]) if(el[k]!==String(x)) el[k]=String(x);
      if(el.value!==String(n.value)) el.value=String(n.value);
      set(el,"aria-valuetext",null);
    }
    if(isField(el)) this.field(el,n);
  }
  make(t,n,kind,v) {
    let el;
    if(kind==="textarea") el=document.createElement("textarea");
    else if(kind==="div") el=document.createElement("div");
    else {
      el=document.createElement("input");
      el.type=kind;
      if(kind!=="range") {el.autocomplete="off";el.setAttribute("autocapitalize","off");}
    }
    if(kind!=="div") el.spellcheck=false;
    el.dataset.kind=kind; el.dataset.key=n.id;
    el.id="jfa-"+t.win+"-"+n.id.replace(".","-");
    v.els.set(n.id,el);
    return el;
  }
  // a text field: the app's text and (while it has the focus) its selection
  field(el,n) {
    el.readOnly=!!(n.state&(S.READONLY|S.DISABLED));
    if(this.composing===el) return; // the IME's text stays until it is committed
    // an edit of the DOM on its way to the app: older updates do not undo it
    const p=this.pending.get(el);
    if(p) {if(n.text!==p.text && performance.now()<p.until) return; this.pending.delete(el);}
    if(el.value!==n.text) el.value=n.text;
    if(n.caret>=0) {
      const a=charsToUnits(n.text,n.anchor>=0?n.anchor:n.caret), c=charsToUnits(n.text,n.caret);
      const sel=[Math.min(a,c),Math.max(a,c),c<a?"backward":"forward"];
      const old=this.synced.get(el);
      if(!old || old[0]!==sel[0] || old[1]!==sel[1] || old[2]!==sel[2] || el.selectionStart!==sel[0] || el.selectionEnd!==sel[1]) {
        this.synced.set(el,sel);
        try {el.setSelectionRange(sel[0],sel[1],sel[2]);} catch {}
      }
    }
    // the IME places its window at the field's own caret: lines as high as the app's
    const h=n.caretRect[3]/devicePixelRatio;
    if(h>0) {const fs=Math.round(h*0.8)+"px",lh=h+"px";if(el.style.fontSize!==fs){el.style.fontSize=fs;el.style.lineHeight=lh;}}
  }
  // the DOM parent of every node of window t, in tree order
  arrange(t) {
    const v=this.wins.get(t.win);
    for(const [id,el] of v.els) if(!t.nodes.has(id)) {el.remove();v.els.delete(id);if(this.tabbed===el)this.tabbed=null;}
    const last=new Map();
    for(const id of t.order) {
      const el=v.els.get(id);
      if(!el) continue;
      const box=this.holder(t,id), parent=box ? v.els.get(box) : v.el, prev=last.get(parent) || null;
      if(el.parentNode!==parent || el.previousElementSibling!==prev) parent.insertBefore(el,prev?prev.nextSibling:parent.firstChild);
      last.set(parent,el);
    }
    // openers point at what they opened
    const opened=new Map();
    for(const id of t.order) {
      let p=t.parent.get(id);
      if(p===null || p===undefined) continue;
      const pn=t.nodes.get(p);
      if(pn && !holds(pn) && !opened.has(p)) opened.set(p,v.els.get(id)?.id);
    }
    for(const [id,el] of v.els) set(el,"aria-controls",opened.get(id) || null);
  }
  // the nearest ancestor of node id that holds elements (null: the window)
  holder(t,id) {
    for(let p=t.parent.get(id);p!==null && p!==undefined;p=t.parent.get(p)) if(holds(t.nodes.get(p))) return p;
    return null;
  }
  // every element at its node's rectangle (CSS px, relative to its DOM parent)
  place() {
    const k=1/devicePixelRatio;
    for(const t of this.mirror.wins.values()) {
      const v=this.wins.get(t.win), [wx,wy,ww,wh]=t.place;
      Object.assign(v.el.style,{left:wx*k+"px",top:wy*k+"px",width:ww*k+"px",height:wh*k+"px"});
      for(const [id,el] of v.els) {
        const n=t.nodes.get(id), box=this.holder(t,id), o=box ? t.nodes.get(box).rect : [0,0];
        const [x,y,w,h]=n.rect;
        const s=el.style, l=(x-o[0])*k+"px", tp=(y-o[1])*k+"px", wd=Math.max(1,w)*k+"px", ht=Math.max(1,h)*k+"px";
        if(s.left!==l) s.left=l; if(s.top!==tp) s.top=tp; if(s.width!==wd) s.width=wd; if(s.height!==ht) s.height=ht;
      }
    }
  }
  /** the device pixel ratio changed: every element anew */
  resized() {this.place();}
  // the focused node's element (of the window with the keyboard)
  target() {
    const t=this.mirror.active();
    return t && t.focus ? this.wins.get(t.win)?.els.get(t.focus) || null : null;
  }
  // roving tabindex: the focus (else the first focusable node) can be reached with Tab; the
  // DOM focus follows the app's while it is ours (lost: an element that moved, refocus)
  focus(lost) {
    let el=this.target();
    const tab=el || this.root.querySelector("[tabindex]");
    if(tab!==this.tabbed) {
      if(this.tabbed?.isConnected) this.tabbed.tabIndex=-1;
      this.tabbed=tab; if(tab) tab.tabIndex=0;
    }
    if(!this.follow && !lost) return;
    const a=document.activeElement, to=el || this.input;
    const ours=lost || a===this.input || a===document.body || a===null || this.root.contains(a);
    if(!ours || a===to) return;
    this.own=true;
    try {to.focus({preventScroll:true});} finally {this.own=false;}
  }
  // the node of an element (and the element that stands for it)
  nodeOf(el) {
    const e=el?.closest?.("[data-key]");
    if(!e || !this.root.contains(e)) return null;
    const n=this.mirror.wins.get(Number(e.closest("[data-win]")?.dataset.win))?.nodes.get(e.dataset.key);
    return n ? {n,el:e} : null;
  }
  act(n,act,a=0,b=0,value=0,text="") {
    this.send({win:n.win,lo:n.key.lo,hi:n.key.hi,act,a,b,value,text});
  }
  // a click of assistive technology (the overlay takes no pointer): press, toggle, select
  clicked(e) {
    const f=this.nodeOf(e.target);
    if(!f) return;
    e.preventDefault();
    if(isField(f.el)) return;
    if(f.n.actions&A.CLICK && !(f.n.state&S.DISABLED)) this.act(f.n,ACT.CLICK);
    else if(f.n.actions&A.FOCUS) this.act(f.n,ACT.FOCUS);
  }
  // the DOM focus came to an element (Tab, a screen reader): the app's focus follows
  focused(e) {
    if(this.own) return;
    this.follow=true;
    const f=this.nodeOf(e.target);
    if(!f) return;
    const t=this.mirror.wins.get(f.n.win);
    if(f.n.actions&A.FOCUS && !(t.active && t.focus===f.n.id)) this.act(f.n,ACT.FOCUS);
  }
  /** (loader: keys and text for a shadow element) the app's focus goes there first when it is
   * elsewhere (a focus event the browser held back while the page had no system focus) */
  claim(el) {
    const f=this.nodeOf(el);
    if(!f || this.own) return;
    const t=this.mirror.wins.get(f.n.win);
    if(f.n.actions&A.FOCUS && !(t.active && t.focus===f.n.id) && this.claimed!==f.el) {this.claimed=f.el;this.act(f.n,ACT.FOCUS);}
  }
  /** (loader: input events that were not prevented) a text field edited in the DOM (a
   * replacement, a virtual keyboard, assistive technology) or a range moved; type: the
   * event's inputType */
  edited(el,type="") {
    const f=this.nodeOf(el);
    if(!f) return;
    const n=f.n;
    if(f.el.type==="range") {
      if(n.state&S.DISABLED || !(n.actions&A.VALUE)) {f.el.value=String(n.value);return;}
      this.act(n,ACT.VALUE,0,0,Number(f.el.value));
      return;
    }
    // a composition's text went to the app with compositionend; Firefox sends its last input
    // after that (isComposing false), when the field shows the app's text again: no edit
    if(!isField(f.el) || this.composing===f.el || type.endsWith("CompositionText")) return;
    if(n.state&(S.DISABLED|S.READONLY) || !(n.actions&A.TEXT)) {this.restore(f.el);return;}
    const text=f.el.value;
    this.act(n,ACT.TEXT,0,0,0,text);
    const s=f.el.selectionStart, e=f.el.selectionEnd, back=f.el.selectionDirection==="backward";
    this.act(n,ACT.SELECT,unitsToChars(text,back?e:s),unitsToChars(text,back?s:e));
    this.pending.set(f.el,{text,until:performance.now()+1500});
  }
  /** (loader: composition end) the field shows the app's text again */
  restore(el) {
    if(this.composing===el) this.composing=null;
    const f=this.nodeOf(el);
    if(f) {this.synced.delete(f.el);this.field(f.el,f.n);}
  }
  // the DOM selection of a focused text field moved without us (assistive technology)
  selected() {
    const el=document.activeElement;
    if(!el || !isField(el) || this.composing===el || this.pending.has(el) || !this.root.contains(el)) return;
    const f=this.nodeOf(el), old=this.synced.get(el);
    if(!f || f.n.caret<0 || el.value!==f.n.text) return; // DOM offsets of another text
    const sel=[el.selectionStart,el.selectionEnd,el.selectionDirection==="backward"?"backward":"forward"];
    if(old && old[0]===sel[0] && old[1]===sel[1] && (sel[0]===sel[1] || old[2]===sel[2])) return;
    this.synced.set(el,sel);
    const text=f.n.text, back=sel[2]==="backward";
    this.act(f.n,ACT.SELECT,unitsToChars(text,back?sel[1]:sel[0]),unitsToChars(text,back?sel[0]:sel[1]));
  }
}
