import {noRoute, requiredText} from '../../runtime/http.js';

export function createHandler() {
  const messages = [];
  return async ({method, path, body}) => {
    if (method === 'GET' && path === '/messages') return {messages: [...messages]};
    if (method === 'POST' && path === '/messages') {
      const message = {
        type: requiredText(body?.type, 'type'), customerId: requiredText(body?.customerId, 'customerId'),
        orderId: requiredText(body?.orderId, 'orderId'), delivered: false,
      };
      messages.push(message);
      return {accepted: true, delivered: false};
    }
    return noRoute();
  };
}
