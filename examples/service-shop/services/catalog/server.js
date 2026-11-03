import {HttpError, noRoute, requiredText} from '../../runtime/http.js';

const products = [
  {sku: 'coffee', name: 'Demo coffee', category: 'coffee', baseCents: 2000},
  {sku: 'mug', name: 'Demo mug', category: 'accessories', baseCents: 1200},
];

export function createHandler({call}) {
  return async ({method, path, query}) => {
    // This read endpoint terminates the promotions → catalog back-query.
    if (method === 'GET' && path === '/categories') {
      const sku = requiredText(query.get('sku'), 'sku');
      const product = products.find(product => product.sku === sku);
      if (!product) throw new HttpError(404, 'Unknown product');
      return {sku, category: product.category};
    }
    if (method === 'GET' && path === '/products') {
      const result = [];
      for (const product of products) {
        const quote = await call('PRICING_API', `/quote?sku=${product.sku}&baseCents=${product.baseCents}`);
        const stock = await call('INVENTORY_API', `/stock?sku=${product.sku}`);
        result.push({sku: product.sku, name: product.name, category: product.category, unitPriceCents: quote.unitPriceCents, available: stock.available});
      }
      return {products: result};
    }
    return noRoute();
  };
}
