// The secret store of the web host (lib/plat/web_secret.rs, docs/SECRETS.md). Per app one
// non-extractable AES-GCM key (256 bits, WebCrypto) kept as a CryptoKey in IndexedDB; the
// values encrypted with it (random 12-byte IV, additional data "app\0service\0account") in the
// same database. It keeps the values from other origins and from someone reading the profile's
// storage; any script of this origin can still use the key.
export const SECRETS_DATABASE="jfgui-secrets-v1";
const encode=new TextEncoder(), decode=new TextDecoder();
const aad=(app,service,account)=>encode.encode(`${app}\0${service}\0${account}`);
function done(transaction) {
  return new Promise((resolve,reject)=>{
    transaction.oncomplete=()=>resolve();
    transaction.onabort=transaction.onerror=()=>reject(transaction.error || new Error("IndexedDB-Transaktion abgebrochen"));
  });
}
function open(factory) {
  return new Promise((resolve,reject)=>{
    const request=factory.open(SECRETS_DATABASE,1);
    request.onupgradeneeded=()=>{
      const db=request.result;
      if(!db.objectStoreNames.contains("keys"))db.createObjectStore("keys",{keyPath:"app"});
      if(!db.objectStoreNames.contains("values"))db.createObjectStore("values",{keyPath:["app","service","account"]}).createIndex("app","app");
    };
    request.onsuccess=()=>resolve(request.result);
    request.onerror=()=>reject(request.error);
    request.onblocked=()=>reject(new Error("Geheimnisspeicher blockiert"));
  });
}
// The app's key: the stored one, else a new one -- added in the same transaction that looked,
// so two pages starting at once agree on one key.
async function appKey(db,subtle,app) {
  const read=db.transaction("keys","readonly"),got=read.objectStore("keys").get(app);
  await done(read);
  if(got.result)return got.result.key;
  const fresh=await subtle.generateKey({name:"AES-GCM",length:256},false,["encrypt","decrypt"]);
  const write=db.transaction("keys","readwrite"),store=write.objectStore("keys");
  let key=fresh;
  const again=store.get(app);
  again.onsuccess=()=>{if(again.result)key=again.result.key;else store.add({app,key:fresh,created:Date.now()});};
  await done(write);
  return key;
}
// The store, or null without IndexedDB / WebCrypto (an insecure context) or with a key that
// does not work. entries: the app's values, decrypted ({app,service,account,value}; value null:
// the record does not decrypt).
export async function openSecrets(app,factory=globalThis.indexedDB,subtle=globalThis.isSecureContext?globalThis.crypto?.subtle:undefined) {
  if(!factory || !subtle)return null;
  const db=await open(factory),keys=new Map();
  const key=async name=>{if(!keys.has(name))keys.set(name,await appKey(db,subtle,name));return keys.get(name);};
  try {
    // the key must work before the app relies on it
    const own=await key(app), iv=crypto.getRandomValues(new Uint8Array(12)), probe=encode.encode("jfgui");
    const back=await subtle.decrypt({name:"AES-GCM",iv},own,await subtle.encrypt({name:"AES-GCM",iv},own,probe));
    if(decode.decode(back)!=="jfgui")throw new Error("Schlüsselprobe fehlgeschlagen");
    const read=db.transaction("values","readonly"),all=read.objectStore("values").index("app").getAll(app);
    await done(read);
    const entries=[];
    for(const r of all.result) {
      let value=null;
      try {value=new Uint8Array(await subtle.decrypt({name:"AES-GCM",iv:r.iv,additionalData:aad(r.app,r.service,r.account)},own,r.data));}
      catch {}
      entries.push({app:r.app,service:r.service,account:r.account,value});
    }
    // One change after the other: a later set must not overtake an earlier one.
    let queue=Promise.resolve();
    async function apply(bytes) {
      const view=new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength);let at=4;
      const block=()=>{const n=view.getUint32(at,true);if(at+4+n>bytes.length)throw new Error("Ungültiger Geheimnisauftrag");const b=bytes.subarray(at+4,at+4+n);at+=4+n;return b;};
      const op=view.getUint32(0,true),owner=decode.decode(block()),service=decode.decode(block()),account=decode.decode(block());
      if(op===1) {
        const value=block(),iv=crypto.getRandomValues(new Uint8Array(12));
        const data=new Uint8Array(await subtle.encrypt({name:"AES-GCM",iv,additionalData:aad(owner,service,account)},await key(owner),value));
        const t=db.transaction("values","readwrite");t.objectStore("values").put({app:owner,service,account,iv,data});
        await done(t);
      } else if(op===2) {
        const t=db.transaction("values","readwrite");t.objectStore("values").delete([owner,service,account]);
        await done(t);
      } else throw new Error("Ungültiger Geheimnisauftrag");
    }
    const change=bytes=>{const next=queue.then(()=>apply(bytes)).finally(()=>bytes.fill(0));queue=next.catch(()=>{});return next;};
    return {entries,change};
  } catch(error) {db.close();throw error;}
}
// The worker's jfgui_secrets message: u32 count, per entry app, service, account, u32
// readable, value (u32 length + bytes each).
export function encodeSecrets(entries) {
  const parts=entries.map(e=>[encode.encode(e.app),encode.encode(e.service),encode.encode(e.account),e.value || new Uint8Array(0)]);
  const data=new Uint8Array(4+parts.reduce((n,[a,s,c,v])=>n+20+a.length+s.length+c.length+v.length,0)),view=new DataView(data.buffer);
  view.setUint32(0,parts.length,true);let at=4;
  const put=b=>{view.setUint32(at,b.length,true);data.set(b,at+4);at+=4+b.length;};
  parts.forEach(([a,s,c,v],i)=>{put(a);put(s);put(c);view.setUint32(at,entries[i].value?1:0,true);at+=4;put(v);});
  return data;
}
