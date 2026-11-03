import {HttpError, noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler() {
  const stock = new Map([['coffee', 20], ['mug', 10]]);
  const reservations = new Map();
  return async ({method, path, query, body}) => {
    if (method === 'GET' && path === '/stock') {
      const sku = requiredText(query.get('sku'), 'sku');
      if (!stock.has(sku)) throw new HttpError(404, 'Unknown product');
      return {sku, available: stock.get(sku)};
    }
    if (method === 'POST' && path === '/reserve') {
      const orderId = requiredText(body?.orderId, 'orderId');
      const sku = requiredText(body?.sku, 'sku');
      const quantity = positiveInteger(body?.quantity, 'quantity');
      if (reservations.has(orderId)) {
        const previous = reservations.get(orderId);
        if (previous.sku !== sku || previous.quantity !== quantity) throw new HttpError(409, 'Conflicting reservation');
        return previous;
      }
      if (!stock.has(sku)) throw new HttpError(404, 'Unknown product');
      if (stock.get(sku) < quantity) throw new HttpError(409, 'Insufficient stock');
      stock.set(sku, stock.get(sku) - quantity);
      const reservation = {orderId, sku, quantity, status: 'reserved'};
      reservations.set(orderId, reservation);
      return reservation;
    }
    return noRoute();
  };
}
