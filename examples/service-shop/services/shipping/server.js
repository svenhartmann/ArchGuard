import {noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, body}) => {
    if (method === 'POST' && path === '/shipments') {
      const orderId = requiredText(body?.orderId, 'orderId');
      const customerId = requiredText(body?.customerId, 'customerId');
      const sku = requiredText(body?.sku, 'sku');
      const quantity = positiveInteger(body?.quantity, 'quantity');
      const stock = await call('INVENTORY_API', `/stock?sku=${encodeURIComponent(sku)}`);
      await call('NOTIFICATIONS_API', '/messages', {method: 'POST', body: {type: 'shipment-scheduled', customerId, orderId}});
      return {id: `shipment-${orderId}`, orderId, sku, quantity, status: 'scheduled', remainingStock: stock.available, demo: true};
    }
    return noRoute();
  };
}
