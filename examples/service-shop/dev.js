// Run the ten demo APIs locally; dependency wiring comes from the Compose analysis.
import {execFileSync} from 'node:child_process';
import {fileURLToPath,pathToFileURL} from 'node:url';
import path from 'node:path';
import {createClient,createServer,HttpError} from './runtime/http.js';

const root=fileURLToPath(new URL('.',import.meta.url));

export async function startShop(topology,{port=8890}={}){
  // Recorded traces always mark the topology partial; only Compose gaps block the demo.
  if(!topology.complete&&topology.diagnostics.some(d=>d.file!=='.archguard/otel-traces.json'))throw new Error('The demo requires complete Compose endpoint analysis.');
  const services=topology.services.filter(service=>service.build_context);
  const servers=[],urls=new Map(),handlers=new Map();
  const close=async()=>{
    await Promise.all(servers.map(server=>new Promise((resolve,reject)=>{
      server.close(error=>error?reject(error):resolve());server.closeAllConnections();
    })));
  };
  try{
    for(const service of services){
      const server=createServer(request=>{
        const handler=handlers.get(service.id);
        if(!handler)throw new HttpError(503,'Demo is starting');
        return handler(request);
      },service.name);
      await new Promise((resolve,reject)=>{
        server.once('error',reject);
        server.listen(service.name==='gateway'?port:0,'127.0.0.1',()=>{server.off('error',reject);resolve();});
      });
      servers.push(server);
      urls.set(service.id,'http://127.0.0.1:'+server.address().port);
    }
    for(const service of services){
      const env={};
      for(const edge of topology.dependencies.filter(d=>d.from===service.id&&d.kind==='configured_http')){
        if(!urls.has(edge.to))throw new Error('Demo HTTP target has no local implementation: '+edge.to);
        for(const evidence of edge.evidence){
          const match=evidence.pointer.match(/\/environment\/([^/]+)$/);
          if(!match)throw new Error('Demo endpoint needs a named environment key.');
          env[match[1].replace(/~1/g,'/').replace(/~0/g,'~')]=urls.get(edge.to);
        }
      }
      const {createHandler}=await import(pathToFileURL(path.join(root,service.build_context,'server.js')));
      handlers.set(service.id,createHandler({call:createClient(env)}));
    }
    return {urls,gateway:urls.get(services.find(s=>s.name==='gateway').id),close};
  }catch(error){await close();throw error;}
}

export function loadTopology(){
  const binary=process.env.ARCHGUARD_BIN||path.resolve(root,'../../target/debug/archguard');
  return JSON.parse(execFileSync(binary,['services',root,'--format','json'],{encoding:'utf8'}));
}

if(process.argv[1]&&pathToFileURL(path.resolve(process.argv[1])).href===import.meta.url){
  try{
    const shop=await startShop(loadTopology(),{port:Number(process.env.PORT||8890)});
    console.log('Shop demo: '+shop.gateway+' — ten local APIs; mock payments, notifications and in-memory data.');
    console.log('GET /products · GET /availability?sku=coffee · POST /checkout');
    for(const signal of ['SIGINT','SIGTERM'])process.once(signal,()=>shop.close().then(()=>process.exit(0)));
  }catch(error){console.error(error.message);process.exitCode=1;}
}
