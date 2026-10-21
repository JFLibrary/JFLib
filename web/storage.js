// Versioned storage survives the public namespace rename. Legacy records are never deleted.
// A migration is acknowledged only after the destination transaction has committed.
export const FILES_DATABASE="jfgui-files-v1";
const LEGACY_DATABASE="asmgui-files-v1";
function open(factory,name,existingOnly=false) {
  return new Promise((resolve,reject)=>{
    let absent=false;const request=factory.open(name,1);
    request.onupgradeneeded=()=>{
      if(existingOnly){absent=true;request.transaction.abort();}
      else if(!request.result.objectStoreNames.contains("files"))request.result.createObjectStore("files",{keyPath:"key"});
    };
    request.onsuccess=()=>resolve(request.result);
    request.onerror=()=>absent?resolve(null):reject(request.error);
  });
}
function records(database) {
  if(!database || !database.objectStoreNames.contains("files"))return Promise.resolve([]);
  return new Promise((resolve,reject)=>{
    const transaction=database.transaction("files","readonly"),request=transaction.objectStore("files").getAll();
    transaction.oncomplete=()=>resolve(request.result);
    transaction.onabort=transaction.onerror=()=>reject(transaction.error || request.error || new Error("Dateiliste nicht gelesen"));
  });
}
function copyMissing(database,legacy) {
  if(!legacy.length)return Promise.resolve();
  return new Promise((resolve,reject)=>{
    const transaction=database.transaction("files","readwrite"),store=transaction.objectStore("files");
    transaction.oncomplete=resolve;
    transaction.onabort=transaction.onerror=()=>reject(transaction.error || new Error("Datenübernahme abgebrochen"));
    for(const record of legacy){
      const request=store.get(record.key);
      request.onsuccess=()=>{if(request.result===undefined)store.put(record);};
    }
  });
}
// Current records take precedence. On quota/access failure existing data remains readable,
// while a null database disables durable writes until the migration can be retried.
export async function openFilesStorage(factory=globalThis.indexedDB) {
  const database=await open(factory,FILES_DATABASE);let legacy=null;
  let old=[],current=[];
  try {
    current=await records(database);
    const names=typeof factory.databases==="function"?await factory.databases():null;
    if(names===null || names.some(d=>d.name===LEGACY_DATABASE))legacy=await open(factory,LEGACY_DATABASE,true);
    old=await records(legacy);
    await copyMissing(database,old);
    return {database,entries:await records(database),error:null};
  }catch(error){
    database.close();
    const combined=new Map(old.map(record=>[record.key,record]));
    for(const record of current)combined.set(record.key,record);
    return {database:null,entries:[...combined.values()],error};
  }finally{legacy?.close();}
}
