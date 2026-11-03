import {HttpError, noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  const orders = [];
  let nextId = 1;
  return async ({method, path, query, body}) => {
    // Independent read endpoint: risk → orders ends here, never starts checkout.
    if (method === 'GET' && path === '/customer-history') {
      const customerId = requiredText(query.get('customerId'), 'customerId');
      return {customerId, orders: orders.filter(order => order.customerId === customerId).map(({id, totalCents}) => ({id, totalCents}))};
    }
    if (method === 'POST' && path === '/checkout') {
      const customerId = requiredText(body?.customerId, 'customerId');
      const sku = requiredText(body?.sku, 'sku');
      const quantity = positiveInteger(body?.quantity, 'quantity');
      const unitPriceCents = positiveInteger(body?.unitPriceCents, 'unitPriceCents');
      const totalCents = positiveInteger(unitPriceCents * quantity, 'totalCents');
      const stock = await call('INVENTORY_API', `/stock?sku=${encodeURIComponent(sku)}`);
      if (stock.available < quantity) throw new HttpError(409, 'Insufficient stock');
      const id = `order-${nextId++}`;
      const payment = await call('PAYMENTS_API', '/authorize', {method: 'POST', body: {orderId: id, customerId, amountCents: totalCents}});
      await call('INVENTORY_API', '/reserve', {method: 'POST', body: {orderId: id, sku, quantity}});
      const shipment = await call('SHIPPING_API', '/shipments', {method: 'POST', body: {orderId: id, customerId, sku, quantity}});
      await call('NOTIFICATIONS_API', '/messages', {method: 'POST', body: {type: 'order-confirmed', customerId, orderId: id}});
      const order = {id, customerId, sku, quantity, totalCents, status: 'confirmed', payment, shipment};
      orders.push(order);
      return order;
    }
    return noRoute();
  };
}
