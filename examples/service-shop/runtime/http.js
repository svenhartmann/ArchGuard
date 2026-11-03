import {createServer as httpServer} from 'node:http';

export class HttpError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

export function requiredText(value, name) {
  if (typeof value !== 'string' || !value.trim() || value.length > 120) throw new HttpError(400, `${name} is required (1–120 characters)`);
  return value;
}

export function positiveInteger(value, name) {
  if (!Number.isSafeInteger(value) || value < 1) throw new HttpError(400, `${name} must be a positive integer`);
  return value;
}

export function noRoute() { throw new HttpError(404, 'Unknown demo API route'); }

// Each environment key is a Compose-configured service boundary. Calls in the
// handlers use these same keys, rather than maintaining a second endpoint map.
export function createClient(env, fetcher = fetch) {
  return async (key, path, {method = 'GET', body} = {}) => {
    if (!env[key]) throw new HttpError(503, `Missing service configuration: ${key}`);
    try {
      const response = await fetcher(new URL(path, env[key]), {
        method, headers: {'content-type': 'application/json'},
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(3000),
      });
      const result = await response.json();
      if (!response.ok) throw new HttpError(response.status, result.error || 'Downstream request rejected');
      return result;
    } catch (error) {
      if (error instanceof HttpError) throw error;
      throw new HttpError(502, `Service unavailable: ${key}`);
    }
  };
}

export function createServer(handler) {
  const server = httpServer(async (request, response) => {
    response.setHeader('content-type', 'application/json; charset=utf-8');
    try {
      const url = new URL(request.url, 'http://demo');
      if (request.method === 'GET' && url.pathname === '/health') {
        response.end(JSON.stringify({status: 'ok', demo: true}));
        return;
      }
      let size = 0;
      const chunks = [];
      for await (const chunk of request) {
        const bytes = Buffer.from(chunk);
        size += bytes.length;
        if (size <= 65536) chunks.push(bytes);
      }
      if (size > 65536) throw new HttpError(413, 'JSON body exceeds 64 KiB');
      let body;
      if (size) {
        if (!/^application\/json(?:\s*;|$)/i.test(request.headers['content-type'] || '')) throw new HttpError(415, 'Use application/json');
        try { body = JSON.parse(Buffer.concat(chunks).toString('utf8')); }
        catch { throw new HttpError(400, 'Invalid JSON body'); }
      }
      const result = await handler({method: request.method, path: url.pathname, query: url.searchParams, body});
      response.end(JSON.stringify(result));
    } catch (error) {
      response.statusCode = error instanceof HttpError ? error.status : 500;
      response.end(JSON.stringify({error: error instanceof HttpError ? error.message : 'Internal demo service error'}));
    }
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  return server;
}
