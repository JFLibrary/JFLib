// System notifications for plat::tray::notify (lib/plat/web_notify.rs): the Notification API of
// the page.  The worker's REQUEST_NOTIFY (u32 op 1 show / 2 close, u32 id; show: title, body as
// u32 length + UTF-8, u32 w, u32 h, w*h R, G, B, A) arrives within the activation of the
// user's gesture that led to it, so a permission that was not asked for yet is asked for here
// (Notification.requestPermission).  A click focuses the tab and reports the id as clicked, a
// close (the user, the system, refused) as gone: {type:"note", id, clicked} to the worker
// (jfgui_note).
const decoder=new TextDecoder();

export function notifyState() {
  return typeof Notification==="undefined" ? "unsupported" : Notification.permission || "default";
}

function iconUrl(w,h,px) {
  if(!w || !h) return undefined;
  try {
    const canvas=document.createElement("canvas");canvas.width=w;canvas.height=h;
    canvas.getContext("2d").putImageData(new ImageData(new Uint8ClampedArray(px),w,h),0,0);
    return canvas.toDataURL("image/png");
  } catch {return undefined;}
}

export class Notes {
  // send: a message to the worker; changed: the permission may have changed
  constructor(send,changed) {this.send=send;this.changed=changed;this.open=new Map();}
  async request(buffer) {
    const data=new Uint8Array(buffer), v=new DataView(buffer);let at=0;
    const u32=()=>{const x=v.getUint32(at,true);at+=4;return x;};
    const str=()=>{const n=u32();const t=decoder.decode(data.subarray(at,at+n));at+=n;return t;};
    const op=u32(), id=u32();
    if(op===2) {const n=this.open.get(id);this.open.delete(id);n?.close();return;}
    const title=str(), body=str(), w=u32(), h=u32();
    const px=data.subarray(at,at+w*h*4);
    if(typeof Notification==="undefined") {this.send({type:"note",id,clicked:false});return;}
    let permission=Notification.permission;
    if(permission==="default") {
      try {permission=await Notification.requestPermission();} catch {permission=Notification.permission;}
      this.changed();
    }
    if(permission!=="granted") {this.send({type:"note",id,clicked:false});return;}
    let note;
    try {note=new Notification(title,{body,icon:iconUrl(w,h,px),tag:`jfgui-${id}`});}
    catch(error) {console.warn("Benachrichtigung:",error);this.send({type:"note",id,clicked:false});return;}
    this.open.set(id,note);
    note.onclick=e=>{
      e?.preventDefault?.();
      try {window.focus();} catch {}
      // reported once: clicked; the close that follows is not reported again
      if(this.open.delete(id)) this.send({type:"note",id,clicked:true});
      note.close();
    };
    note.onclose=()=>{if(this.open.delete(id)) this.send({type:"note",id,clicked:false});};
    note.onerror=()=>{if(this.open.delete(id)) this.send({type:"note",id,clicked:false});};
  }
}
