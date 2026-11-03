import {noRoute, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, query}) => {
    if (method === 'GET' && path === '/eligibility') {
      const sku = requiredText(query.get('sku'), 'sku');
      // The price request already originated in catalog, but this API contract
      // drops its category and forces promotions to query catalog again.
      const {category} = await call('CATALOG_API', `/categories?sku=${encodeURIComponent(sku)}`);
      return {sku, discountPercent: category === 'coffee' ? 10 : 0};
    }
    return noRoute();
  };
}
