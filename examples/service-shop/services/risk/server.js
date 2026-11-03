import {noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, body}) => {
    if (method === 'POST' && path === '/assess') {
      const customerId = requiredText(body?.customerId, 'customerId');
      const amountCents = positiveInteger(body?.amountCents, 'amountCents');
      // A synchronous history back-query couples payments to orders availability.
      const history = await call('ORDERS_API', `/customer-history?customerId=${encodeURIComponent(customerId)}`);
      return {approved: amountCents <= 100000 || history.orders.length > 0, previousOrders: history.orders.length, demoRule: true};
    }
    return noRoute();
  };
}
