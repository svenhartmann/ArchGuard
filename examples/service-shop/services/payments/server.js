import {HttpError, noRoute, positiveInteger, requiredText} from '../../runtime/http.js';

export function createHandler({call}) {
  return async ({method, path, body}) => {
    if (method === 'POST' && path === '/authorize') {
      const orderId = requiredText(body?.orderId, 'orderId');
      const customerId = requiredText(body?.customerId, 'customerId');
      const amountCents = positiveInteger(body?.amountCents, 'amountCents');
      const assessment = await call('RISK_API', '/assess', {method: 'POST', body: {customerId, amountCents}});
      if (!assessment.approved) throw new HttpError(409, 'Demo risk check rejected payment');
      return {id: `mock-payment-${orderId}`, status: 'authorized', amountCents, mock: true};
    }
    return noRoute();
  };
}
