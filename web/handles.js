// The user's own files through the File System Access API (Chromium: showOpenFilePicker,
// showSaveFilePicker, FileSystemFileHandle.createWritable). Each chosen file is bound to a
// virtual path /handles/ID/NAME of the worker's files (lib/plat/web_files.rs, web.rs); the
// handle is kept in IndexedDB (jfgui-handles-v1, at most KEEP per app), so after a reload the
// same document is read again (with permission: at the start, else reopen from a user gesture)
// and written back. Firefox and Safari have no pickers: supported() is false and the host keeps
// its own dialog, imports and downloads.
export const HANDLES_DATABASE="jfgui-handles-v1";
const KEEP=256;

export const supported=()=>typeof globalThis.showOpenFilePicker==="function" && typeof globalThis.showSaveFilePicker==="function";

function request(r) {return new Promise((resolve,reject)=>{r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error);});}
function done(t) {return new Promise((resolve,reject)=>{t.oncomplete=resolve;t.onabort=t.onerror=()=>reject(t.error || new Error("IndexedDB abgebrochen"));});}

// a path's file name: no separators, never empty
const clean=name=>String(name||"Datei").replace(/[\\/\0]/g,"_") || "Datei";

// The app's remembered handles: {records:[{key,app,path,handle,at}], byPath: Map(path -> record)}.
export async function openHandles(app, factory=globalThis.indexedDB) {
  const open=factory.open(HANDLES_DATABASE,1);
  open.onupgradeneeded=()=>{if(!open.result.objectStoreNames.contains("handles"))open.result.createObjectStore("handles",{keyPath:"key"});};
  const db=await request(open);
  const t=db.transaction("handles","readonly");
  const all=(await request(t.objectStore("handles").getAll())).filter(r=>r.app===app && r.handle);
  const store={db,app,records:all,byPath:new Map(all.map(r=>[r.path,r])),writes:new Map()};
  return store;
}

// The path of a chosen handle: the remembered one when it is the same file, else a new one.
export async function bind(store, handle) {
  for(const r of store.records) {
    try {if(r.handle.kind===handle.kind && await r.handle.isSameEntry(handle)) {r.handle=handle;r.at=Date.now();await save(store,r);return r.path;}}
    catch {}
  }
  let path;
  do {
    const id=Array.from(crypto.getRandomValues(new Uint8Array(4)),b=>b.toString(16).padStart(2,"0")).join("");
    path=`/handles/${id}/${clean(handle.name)}`;
  } while(store.byPath.has(path));
  const r={key:store.app+":"+path,app:store.app,path,handle,at:Date.now()};
  store.records.push(r);store.byPath.set(path,r);
  await save(store,r);
  // the oldest go when there are too many (their paths stop working)
  const old=store.records.sort((a,b)=>b.at-a.at).splice(KEEP);
  if(old.length) {
    const t=store.db.transaction("handles","readwrite");
    for(const o of old) {store.byPath.delete(o.path);t.objectStore("handles").delete(o.key);}
    await done(t);
  }
  return path;
}

async function save(store, r) {
  const t=store.db.transaction("handles","readwrite");
  t.objectStore("handles").put({key:r.key,app:r.app,path:r.path,handle:r.handle,at:r.at});
  await done(t);
}

// Whether the handle may be read ("read") or written ("readwrite"); ask: request it (needs the
// user's gesture: a click or key a moment ago). Browsers without queryPermission (OPFS in
// Firefox) need none.
export async function permitted(handle, mode, ask) {
  if(typeof handle.queryPermission!=="function") return true;
  if(await handle.queryPermission({mode})==="granted") return true;
  return ask && await handle.requestPermission({mode})==="granted";
}

// The file's bytes (permission to read first).
export async function readHandle(handle, ask) {
  if(!await permitted(handle,"read",ask)) throw new DOMException("Kein Lesezugriff auf die Datei","NotAllowedError");
  return (await handle.getFile()).arrayBuffer();
}

// The whole file replaced by bytes, one write after the other per path. The browser writes a
// swap file and puts it in place at close() (Chromium: NAME.crswap): until then, and when
// anything fails, the file keeps its old contents.
export function writeBack(store, path, bytes) {
  const r=store.byPath.get(path);
  if(!r) return Promise.reject(new DOMException("Keine Datei zu "+path,"NotFoundError"));
  const run=async()=>{
    if(!await permitted(r.handle,"readwrite",true)) throw new DOMException("Kein Schreibzugriff auf die Datei","NotAllowedError");
    const w=await r.handle.createWritable();
    try {await w.write(bytes);await w.close();}
    catch(error) {await w.abort().catch(()=>{});throw error;}
  };
  const next=(store.writes.get(path)||Promise.resolve()).catch(()=>{}).then(run);
  store.writes.set(path,next);
  return next;
}

// The pickers' file types from the extensions ("png", "jpg"): one entry, every extension.
export function types(exts) {
  const list=[...new Set(exts.map(e=>String(e).replace(/^\./,"").toLowerCase()).filter(e=>/^[a-z0-9+_-]{1,15}$/.test(e)))];
  return list.length ? [{description:list.map(e=>"."+e).join(", "),accept:{"*/*":list.map(e=>"."+e)}}] : [];
}
