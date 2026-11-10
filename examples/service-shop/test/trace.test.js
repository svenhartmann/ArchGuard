import assert from 'node:assert/strict';
import {mkdtemp, readFile} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {test} from 'node:test';
import {createClient, createServer, HttpError} from '../runtime/http.js';

async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}`;
}

test('a forwarded traceparent links SERVER and CLIENT spans across two services', async t => {
  const dir = await mkdtemp(path.join(tmpdir(), 'shop-trace-'));
  process.env.SHOP_TRACE_DIR = dir;
  t.after(() => delete process.env.SHOP_TRACE_DIR);
  const catalog = createServer(async () => { throw new HttpError(404, 'Unknown product'); }, 'catalog');
  const catalogUrl = await listen(catalog);
  const call = createClient({CATALOG_API: catalogUrl});
  const gateway = createServer(() => call('CATALOG_API', '/products?sku=x'), 'gateway');
  const gatewayUrl = await listen(gateway);
  t.after(() => { catalog.close(); gateway.close(); });

  const traceId = '4bf92f3577b34da6a3ce929d0e0e4736', callerSpan = '00f067aa0ba902b7';
  const response = await fetch(`${gatewayUrl}/checkout`, {headers: {traceparent: `00-${traceId}-${callerSpan}-01`}});
  assert.equal(response.status, 404);

  const spans = (await readFile(path.join(dir, 'traces.jsonl'), 'utf8')).trim().split('\n').map(line => {
    const {resource, scopeSpans: [{spans: [span]}]} = JSON.parse(line).resourceSpans[0];
    return {service: resource.attributes.find(a => a.key === 'service.name').value.stringValue, ...span};
  });
  const find = (service, kind) => spans.find(s => s.service === service && s.kind === kind);
  const [gatewayServer, gatewayClient, catalogServer] = [find('gateway', 2), find('gateway', 3), find('catalog', 2)];
  assert.ok(gatewayServer && gatewayClient && catalogServer, JSON.stringify(spans));
  assert.ok(spans.every(s => s.traceId === traceId), 'one trace across both services');
  assert.equal(gatewayServer.parentSpanId, callerSpan);
  assert.equal(gatewayClient.parentSpanId, gatewayServer.spanId);
  assert.equal(catalogServer.parentSpanId, gatewayClient.spanId);
  assert.equal(catalogServer.status.code, 2, '4xx is recorded as an error');
  assert.ok(catalogServer.attributes.some(a => a.key === 'http.route' && a.value.stringValue === '/products'));
});

test('without SHOP_TRACE_DIR nothing is recorded or forwarded', async t => {
  let seen;
  const server = createServer(() => ({ok: true}), 'catalog');
  const call = createClient({CATALOG_API: await listen(server)}, (url, options) => { seen = options.headers; return fetch(url, options); });
  t.after(() => server.close());
  assert.deepEqual(await call('CATALOG_API', '/products'), {ok: true});
  assert.equal(seen.traceparent, undefined);
});
