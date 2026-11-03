// Exercise the checked-in demo through the real analyzer, viewer and HTTP runtime.
const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm');
const path=require('node:path'),{execFileSync}=require('node:child_process'),{pathToFileURL}=require('node:url');
const root=path.resolve(__dirname,'../..'),example=path.join(root,'examples/service-shop');
const binary=process.argv[2]||path.join(root,'target/debug/archguard');
const topology=JSON.parse(execFileSync(binary,['services',example,'--format','json'],{encoding:'utf8'}));
const harness=fs.readFileSync(path.join(root,'scripts/viewer/test_viewer_state.js'),'utf8').split('const node=(id,name,parent=null,kind=')[0];
const launch=vm.runInNewContext(harness+'\nlaunch;',{require,console,performance,Blob});

async function main(){
  // Start with HTTP: before implementation this fails because no runnable demo exists.
  const {startShop}=await import(pathToFileURL(path.join(example,'dev.js')));
  const shop=await startShop(topology,{port:0});
  try{
    assert.equal(shop.urls.size,10,'all ten actual handlers have a local HTTP listener');
    const response=await fetch(shop.gateway+'/products');
    assert.equal(response.status,200);
    const products=await response.json();
    assert.equal(products.products.find(product=>product.sku==='coffee').unitPriceCents,1800,'catalog/pricing/promotions chain applies the coffee discount');
    const order=await fetch(shop.gateway+'/checkout',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({customerId:'customer-1',sku:'coffee',quantity:1})});
    assert.equal(order.status,200,'checkout traverses orders/payments/risk/history without recursion');
    const result=await order.json();
    assert.equal(result.sku,'coffee');
    assert.equal(result.status,'confirmed');
    assert.equal(result.totalCents,1800);
    assert.equal(result.payment.mock,true);
    assert.equal(result.shipment.status,'scheduled');
    const available=await fetch(shop.gateway+'/availability?sku=coffee');
    assert.equal(available.status,200);
    assert.equal((await available.json()).available,19,'checkout reserved exactly one product');
    const ordersId=topology.services.find(service=>service.name==='orders').id;
    const history=await fetch(shop.urls.get(ordersId)+'/customer-history?customerId=customer-1');
    assert.deepEqual((await history.json()).orders,[{id:result.id,totalCents:1800}]);
    const invalid=await fetch(shop.gateway+'/checkout',{method:'POST',headers:{'content-type':'application/json'},body:'{broken'});
    assert.equal(invalid.status,400,'malformed requests are rejected at the actual HTTP boundary');
    const missing=await fetch(shop.gateway+'/does-not-exist');
    assert.equal(missing.status,404);
  }finally{await shop.close();}

  const app=launch({root:example,nodes:[],edges:[],findings:[],cyclicEdges:[],view:{depth:0,visible:null},services:topology},{library:false});
  assert.equal(app.run('ui.workspace'),'services');
  const tree=()=>app.ids.get('services-tree'),edges=()=>tree().querySelectorAll('g[data-service-edge]');
  assert.equal(tree().querySelectorAll('g[data-service-node]').length,14);
  assert.equal(edges().length,15);
  assert.equal(app.ids.get('services-cycles').querySelectorAll('button').length,2);
  let cyclicEdges=0,redReturns=0;
  for(const edge of edges()){
    const [from,to,kind]=JSON.parse(edge.getAttribute('data-service-edge'));
    const cyclic=topology.cycles.some(cycle=>cycle.kind===kind&&cycle.nodes.includes(from)&&cycle.nodes.includes(to));
    if(cyclic)cyclicEdges++;
    const red=cyclic&&edge.getAttribute('data-service-side')==='right';if(red)redReturns++;
    const color=app.run(red?'C.cycle':'C.edge');
    assert.equal(edge.querySelector('path[data-service-line]').getAttribute('stroke'),color,'only right-hand cycle returns are red without a selected reference');
    assert.equal(edge.querySelector('polygon').getAttribute('fill'),color,'service arrow and line use the same state');
  }
  assert.equal(cyclicEdges,6,'both three-service cycles retain all six directed connections');
  assert.equal(redReturns,2,'each three-service cycle has a single red return in this layout');
  const orders=topology.services.find(s=>s.name==='orders');
  tree().querySelectorAll('g[data-service-node]').find(n=>n.getAttribute('data-service-node')===orders.id).click();
  const filter=(id,value)=>{app.ids.get(id).value=value;app.ids.get(id).dispatch('change');};
  filter('services-direction','incoming');assert.equal(edges().length,2);
  filter('services-direction','outgoing');assert.equal(edges().length,4);
  filter('services-direction','all');app.click('services-cycles-only');assert.equal(edges().length,6);
  app.click('services-cycles-only');filter('services-kind','startup');assert.equal(edges().length,7);
  assert.equal(app.ids.get('services-cycles').querySelectorAll('button').length,0);
  console.log('Shop demo: 14 services, 15 HTTP edges, two separate cycles, seven startup edges; real local HTTP flow and viewer filters passed.');
}
main().catch(error=>{console.error(error);process.exitCode=1;});
