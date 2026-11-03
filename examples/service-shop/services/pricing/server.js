import {noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, query}) => {
    if (method === 'GET' && path === '/quote') {
      const sku = requiredText(query.get('sku'), 'sku');
      const baseCents = positiveInteger(Number(query.get('baseCents')), 'baseCents');
      const promotion = await call('PROMOTIONS_API', `/eligibility?sku=${encodeURIComponent(sku)}`);
      return {sku, unitPriceCents: Math.round(baseCents * (100 - promotion.discountPercent) / 100), discountPercent: promotion.discountPercent};
    }
    return noRoute();
  };
}
