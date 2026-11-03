import assert from 'node:assert/strict';
import {test} from 'node:test';

const names = ['gateway', 'catalog', 'pricing', 'promotions', 'orders', 'payments', 'risk', 'inventory', 'shipping', 'notifications'];

async function shop() {
  const handlers = new Map(), calls = [];
  for (const name of names) {
    const {createHandler} = await import(`../services/${name}/server.js`);
    handlers.set(name, createHandler({call: async (key, path, options = {}) => {
      const target = key.replace(/_API$/, '').toLowerCase();
      assert.ok(handlers.has(target), `unknown service ${target}`);
      calls.push([name, target, path]);
      assert.ok(calls.length < 100, 'a configuration cycle must not recurse forever');
      return invoke(target, path, options);
    }}));
  }
  function invoke(name, path, {method = 'GET', body} = {}) {
    const url = new URL(path, 'http://demo');
    return handlers.get(name)({method, path: url.pathname, query: url.searchParams, body});
  }
  return {invoke, calls};
}

test('a real product and checkout flow exercises both finite cycles and all configured API relations', async () => {
  const {invoke, calls} = await shop();
  const products = await invoke('gateway', '/products');
  const coffee = products.products.find(product => product.sku === 'coffee');
  assert.equal(coffee.unitPriceCents, 1800, 'category promotion reduces the 2000-cent coffee price by 10%');
  assert.equal(coffee.available, 20);
  const availability = await invoke('gateway', '/availability?sku=coffee');
  assert.equal(availability.available, 20);
  const checkout = await invoke('gateway', '/checkout', {method: 'POST', body: {customerId: 'customer-1', sku: 'coffee', quantity: 2}});
  assert.equal(checkout.status, 'confirmed');
  assert.equal(checkout.totalCents, 3600);
  assert.equal(checkout.payment.mock, true, 'no real payment provider is contacted');
  assert.equal(checkout.shipment.status, 'scheduled');
  const history = await invoke('orders', '/customer-history?customerId=customer-1');
  assert.equal(history.orders.length, 1, 'the order becomes visible after checkout succeeds');
  assert.equal(history.orders[0].id, checkout.id);
  const messages = await invoke('notifications', '/messages');
  assert.deepEqual(messages.messages.map(message => message.type).sort(), ['order-confirmed', 'shipment-scheduled']);
  assert.equal(messages.messages.every(message => message.delivered === false), true, 'notifications are captured locally only');
  assert.deepEqual([...new Set(calls.map(([from, to]) => `${from}->${to}`))].sort(), [
    'gateway->catalog', 'gateway->orders', 'gateway->inventory',
    'catalog->pricing', 'catalog->inventory', 'pricing->promotions', 'promotions->catalog',
    'orders->payments', 'orders->inventory', 'orders->shipping', 'orders->notifications',
    'payments->risk', 'risk->orders', 'shipping->inventory', 'shipping->notifications',
  ].sort());
  assert.ok(calls.some(([from, to, path]) => from === 'promotions' && to === 'catalog' && path.startsWith('/categories?')));
  assert.ok(calls.some(([from, to, path]) => from === 'risk' && to === 'orders' && path.startsWith('/customer-history?')));
});

test('invalid and unavailable orders do not charge, ship, or enter history', async () => {
  const {invoke, calls} = await shop();
  for (const quantity of [0, -1, 1.5, '2', 100]) {
    await assert.rejects(invoke('gateway', '/checkout', {method: 'POST', body: {customerId: 'customer-1', sku: 'coffee', quantity}}), error => error.status === 400 || error.status === 409);
  }
  const history = await invoke('orders', '/customer-history?customerId=customer-1');
  assert.deepEqual(history.orders, []);
  assert.equal(calls.some(([, to]) => ['payments', 'shipping', 'notifications'].includes(to)), false);
});

test('risk refuses a large first purchase and orders leaves no successful record', async () => {
  const {invoke, calls} = await shop();
  await assert.rejects(invoke('orders', '/checkout', {method: 'POST', body: {customerId: 'new-customer', sku: 'coffee', quantity: 1, unitPriceCents: 150000}}), error => error.status === 409 && /risk/i.test(error.message));
  assert.deepEqual((await invoke('orders', '/customer-history?customerId=new-customer')).orders, []);
  assert.equal(calls.some(([from, to]) => from === 'risk' && to === 'orders'), true);
  assert.equal(calls.some(([, to]) => to === 'shipping'), false);
});

test('unknown API routes and unknown products fail explicitly', async () => {
  const {invoke} = await shop();
  await assert.rejects(invoke('catalog', '/categories?sku=missing'), error => error.status === 404);
  await assert.rejects(invoke('gateway', '/missing'), error => error.status === 404);
});
