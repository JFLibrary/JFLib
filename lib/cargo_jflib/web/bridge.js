// Optional connection to an explicitly started loopback CLI bridge. No auto-discovery.
export async function connectBridge(agent, {url,token}, onState=()=>{}) {
  const target=new URL(url);
  if(target.protocol!=="http:" || target.hostname!=="127.0.0.1" || target.username || target.password || target.pathname!=="/" || target.search || target.hash)
    throw new Error("Die Agenten-Bridge muss auf http://127.0.0.1:PORT/ laufen.");
  if(typeof token!=="string" || token.length<32) throw new Error("Bridge-Token fehlt.");
  const abort=new AbortController();
  async function request(path,body) {
    const response=await fetch(new URL(path,target),{
      method:body===undefined?"GET":"POST",headers:{Authorization:`Bearer ${token}`,"Content-Type":"application/json"},
      body:body===undefined?undefined:JSON.stringify(body), signal:abort.signal,credentials:"omit",redirect:"error",
    });
    if(!response.ok) throw new Error(`Bridge HTTP ${response.status}`);
    return response.json();
  }
  const {client}=await request("connect",{app:document.body.dataset.app});
  let alive=true;
  const disconnect=async()=>{
    if(!alive)return;alive=false;
    try {await request("disconnect",{client});} finally {abort.abort();onState("Bridge getrennt");}
  };
  (async()=>{
    try {
      while(alive) {
        const {requests}=await request(`poll?client=${encodeURIComponent(client)}`);
        for(const job of requests) {
          let response;
          if(job.closeSession===true) {
            await agent.closeSession(`bridge-${job.session}`);
            await request("result",{client,id:job.id,response:null});
            continue;
          }
          try { response=await agent.request(job.request,{session:`bridge-${job.session}`}); }
          catch(error) {response={jsonrpc:"2.0",id:job.request.id??null,error:{code:-32000,message:error.message}};}
          await request("result",{client,id:job.id,response});
        }
      }
    } catch(error) {if(alive){onState(error.message);alive=false;abort.abort();}}
  })();
  onState("Agenten-Bridge verbunden");
  return disconnect;
}
