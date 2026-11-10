import {AsyncLocalStorage} from 'node:async_hooks';
import {randomBytes} from 'node:crypto';
import {appendFileSync, mkdirSync} from 'node:fs';
import {createServer as httpServer} from 'node:http';
import path from 'node:path';

// Optional tracing: with SHOP_TRACE_DIR set, every request is a SERVER span and
// every downstream call a CLIENT span, appended as OTLP JSON Lines to
// $SHOP_TRACE_DIR/traces.jsonl; W3C traceparent carries the context between services.
const active = new AsyncLocalStorage();
const clock = BigInt(Date.now()) * 1000000n - process.hrtime.bigint();
const now = () => clock + process.hrtime.bigint();
const hex = bytes => randomBytes(bytes).toString('hex');
const text = (key, value) => ({key, value: {stringValue: String(value)}});
const integer = (key, value) => ({key, value: {intValue: String(value)}});

function startSpan(service, kind, parent) {
  if (!process.env.SHOP_TRACE_DIR) return undefined;
  return {service, traceId: parent?.traceId || hex(16), spanId: hex(8), parentSpanId: parent?.spanId, kind, start: now()};
}

function parseTraceparent(header) {
  const match = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/.exec(header || '');
  return match && {traceId: match[1], spanId: match[2]};
}

function endSpan(span, name, status, attributes) {
  if (!span) return;
  const record = {
    traceId: span.traceId, spanId: span.spanId, ...(span.parentSpanId && {parentSpanId: span.parentSpanId}),
    name, kind: span.kind, startTimeUnixNano: String(span.start), endTimeUnixNano: String(now()),
    attributes: status ? [...attributes, integer('http.response.status_code', status)] : attributes,
    status: {code: !status || status >= 400 ? 2 : 0},
  };
  const line = {resourceSpans: [{resource: {attributes: [text('service.name', span.service)]}, scopeSpans: [{scope: {name: 'service-shop/runtime/http.js'}, spans: [record]}]}]};
  mkdirSync(process.env.SHOP_TRACE_DIR, {recursive: true});
  appendFileSync(path.join(process.env.SHOP_TRACE_DIR, 'traces.jsonl'), JSON.stringify(line) + '\n');
}

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
// `caller` names the client span when no request is active (e.g. a browser driving the gateway).
export function createClient(env, fetcher = fetch, caller = 'client') {
  return async (key, target, {method = 'GET', body} = {}) => {
    if (!env[key]) throw new HttpError(503, `Missing service configuration: ${key}`);
    const url = new URL(target, env[key]);
    const parent = active.getStore();
    const span = startSpan(parent?.service || caller, 3, parent);
    let status;
    try {
      const response = await fetcher(url, {
        method, headers: {'content-type': 'application/json', ...(span && {traceparent: `00-${span.traceId}-${span.spanId}-01`})},
        body: body === undefined ? undefined : JSON.stringify(body),
        signal: AbortSignal.timeout(3000),
      });
      status = response.status;
      const result = await response.json();
      if (!response.ok) throw new HttpError(response.status, result.error || 'Downstream request rejected');
      return result;
    } catch (error) {
      if (error instanceof HttpError) throw error;
      throw new HttpError(502, `Service unavailable: ${key}`);
    } finally {
      endSpan(span, method, status, [text('http.request.method', method), text('server.address', url.hostname), integer('server.port', url.port), text('url.full', url.origin + url.pathname)]);
    }
  };
}

export function createServer(handler, service = 'service') {
  const server = httpServer((request, response) => {
    const span = startSpan(service, 2, parseTraceparent(request.headers.traceparent));
    // A malformed URL must not throw outside serve(); serve() answers it.
    const route = URL.canParse(request.url, 'http://demo') ? new URL(request.url, 'http://demo').pathname : '/';
    return active.run(span, () => serve(request, response, handler)).finally(() =>
      endSpan(span, `${request.method} ${route}`, response.statusCode, [text('http.request.method', request.method), text('http.route', route), text('url.path', route)]));
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  return server;
}

async function serve(request, response, handler) {
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
}
