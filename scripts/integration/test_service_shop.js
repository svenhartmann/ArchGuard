// Run the checked-in shop demo from the real analyzer's topology over local HTTP.
const assert=require('node:assert/strict'),path=require('node:path');
const {execFileSync}=require('node:child_process'),{pathToFileURL}=require('node:url');
const root=path.resolve(__dirname,'../..'),example=path.join(root,'examples/service-shop');
const binary=process.argv[2]||path.join(root,'target/debug/archguard');
const topology=JSON.parse(execFileSync(binary,['services',example,'--format','json'],{encoding:'utf8'}));

async function main(){
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
  console.log('Shop demo: ten local HTTP services from the analyzed topology; checkout flow passed.');
}
main().catch(error=>{console.error(error);process.exitCode=1;});
