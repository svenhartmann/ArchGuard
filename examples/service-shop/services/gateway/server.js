import {HttpError, noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, query, body}) => {
    if (method === 'GET' && path === '/products') return call('CATALOG_API', '/products');
    if (method === 'GET' && path === '/availability') return call('INVENTORY_API', `/stock?sku=${encodeURIComponent(requiredText(query.get('sku'), 'sku'))}`);
    if (method === 'POST' && path === '/checkout') {
      const customerId = requiredText(body?.customerId, 'customerId');
      const sku = requiredText(body?.sku, 'sku');
      const quantity = positiveInteger(body?.quantity, 'quantity');
      const {products} = await call('CATALOG_API', '/products');
      const product = products.find(product => product.sku === sku);
      if (!product) throw new HttpError(404, 'Unknown product');
      return call('ORDERS_API', '/checkout', {method: 'POST', body: {customerId, sku, quantity, unitPriceCents: product.unitPriceCents}});
    }
    return noRoute();
  };
}
