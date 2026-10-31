// The system for the worker (lib/plat/web_system.rs, web_power.rs; the notifications: notify.js):
// - the user's settings as "key=value" lines (jfgui_system): prefers-color-scheme,
//   prefers-contrast, forced-colors with the eight system colours of its scheme,
//   prefers-reduced-motion, the default font size against 16 px as the text scale, the
//   permission for notifications, whether the Screen Wake Lock exists -- all of them in the
//   start message, the changed ones after every change (media query listeners, the permission's
//   change event, resize, visibilitychange);
// - the lifecycle: visibilitychange -> E_POWER 2 (hidden) / 3 (visible again) -> UIC_SUSPEND;
// - the screen kept on (REQUEST_POWER, plat::inhibit with INHIBIT_IDLE): the Screen Wake Lock,
//   taken again when the tab is visible again (the browser drops it while hidden);
//   document.body.dataset.wakeLock says held / waiting / released / refused (tests).
import {Notes, notifyState} from "./notify.js";

const QUERIES={dark:"(prefers-color-scheme: dark)",light:"(prefers-color-scheme: light)",more:"(prefers-contrast: more)",
  forced:"(forced-colors: active)",motion:"(prefers-reduced-motion: reduce)"};
// the forced scheme's system colours in the order of plat::HcColors (web_system.rs colors=)
const COLORS=["Canvas","CanvasText","Highlight","HighlightText","ButtonFace","ButtonText","GrayText","LinkText"];
const E_POWER=20, REQUEST_NOTIFY=24, REQUEST_POWER=25;

function rgbHex(text) {
  const m=/rgba?\(\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/.exec(text||"");
  return m ? m.slice(1,4).map(x=>Math.min(255,Number(x)).toString(16).padStart(2,"0")).join("").toUpperCase() : null;
}

export class SystemHost {
  constructor(send) {
    this.send=send; this.started=false; this.sent={}; this.wanted=false; this.lock=null; this.acquiring=false;
    this.notes=new Notes(send,()=>this.update());
    this.media=Object.fromEntries(Object.entries(QUERIES).map(([k,q])=>[k,matchMedia(q)]));
    for(const m of Object.values(this.media)) m.addEventListener?.("change",()=>this.update());
    navigator.permissions?.query({name:"notifications"}).then(s=>{s.addEventListener("change",()=>this.update());}).catch(()=>{});
    document.addEventListener("visibilitychange",()=>this.visibility());
    addEventListener("resize",()=>this.update());
  }
  // a probe element: computed sizes and colours of keywords
  probe(style) {
    const el=document.createElement("span");
    el.style.cssText="position:absolute;visibility:hidden;pointer-events:none;forced-color-adjust:none;"+style;
    (document.body||document.documentElement).append(el);
    return el;
  }
  // the browser's default font size (font-size: medium) against 16 px, in percent
  scale() {
    const el=this.probe("font-size:medium");
    const px=parseFloat(getComputedStyle(el).fontSize)||16;el.remove();
    return Math.max(25,Math.min(500,Math.round(px/16*100)));
  }
  colors() {
    const el=this.probe(""), out=[];
    for(const c of COLORS) {el.style.color=c;out.push(rgbHex(getComputedStyle(el).color));}
    el.remove();
    return out.every(Boolean) ? out.join(",") : "";
  }
  values() {
    // evaluated anew (a kept MediaQueryList may answer its last value until its event fired)
    const m=k=>matchMedia(QUERIES[k]).matches, forced=m("forced");
    return {dark:m("dark")?"1":m("light")?"0":"-",contrast:m("more")?"1":"0",forced:forced?"1":"0",colors:forced?this.colors():"",
      motion:m("motion")?"1":"0",scale:String(this.scale()),notify:notifyState(),wakelock:navigator.wakeLock?"1":"0"};
  }
  // the lines of every key (all) or of the changed ones
  text(all) {
    const v=this.values(), lines=[];
    for(const [k,x] of Object.entries(v)) if(all || this.sent[k]!==x) lines.push(`${k}=${x}`);
    this.sent=v;
    return lines.join("\n");
  }
  // for the start message: every key
  snapshot() {return this.text(true);}
  update() {if(!this.started)return;const text=this.text(false);if(text)this.send({type:"system",text});}
  ready() {this.started=true;this.update();}
  visibility() {
    if(this.started) this.send({type:"event",event:[E_POWER,0,0,0,0,0,document.hidden?2:3]});
    if(!document.hidden && this.wanted) this.acquire();
    this.update();
  }
  async acquire() {
    if(this.lock || this.acquiring || !this.wanted || document.hidden || !navigator.wakeLock) return;
    this.acquiring=true;
    try {
      const lock=await navigator.wakeLock.request("screen");
      if(!this.wanted) {lock.release().catch(()=>{});return;}
      this.lock=lock;document.body.dataset.wakeLock="held";
      lock.addEventListener("release",()=>{if(this.lock===lock){this.lock=null;document.body.dataset.wakeLock=this.wanted?"waiting":"released";}});
    } catch(error) {document.body.dataset.wakeLock="refused";console.warn("Bildschirm an:",error?.message||error);}
    finally {this.acquiring=false;}
  }
  power(on) {
    this.wanted=on;
    if(on) {document.body.dataset.wakeLock="waiting";this.acquire();return;}
    const lock=this.lock;this.lock=null;document.body.dataset.wakeLock="released";
    lock?.release().catch(()=>{});
  }
  // the worker's requests of web_notify.rs / web_power.rs -> true when it was one of them
  service(m) {
    if(m.kind===REQUEST_NOTIFY) this.notes.request(m.data).catch(error=>console.warn("Benachrichtigung:",error));
    else if(m.kind===REQUEST_POWER) this.power(new Uint8Array(m.data)[0]===1);
    else return false;
    return true;
  }
}
