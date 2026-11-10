// Record a few real request traces of the local shop into .archguard/otel-traces.json
// (OTLP JSON, read by `archguard services` and the arc42 chapter 6 sequence diagrams).
import assert from 'node:assert/strict';
import {mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {loadTopology, startShop} from '../dev.js';
import {createClient} from '../runtime/http.js';

const root = fileURLToPath(new URL('..', import.meta.url));
const dir = mkdtempSync(path.join(tmpdir(), 'shop-traces-'));
process.env.SHOP_TRACE_DIR = dir;
const shop = await startShop(loadTopology(), {port: 0});
try {
  const browser = createClient({GATEWAY_API: shop.gateway}, fetch, 'browser');
  const checkout = body => browser('GATEWAY_API', '/checkout', {method: 'POST', body});
  const status = promise => promise.then(() => 200, error => error.status);
  await browser('GATEWAY_API', '/products');
  assert.equal((await checkout({customerId: 'customer-1', sku: 'coffee', quantity: 2})).status, 'confirmed');
  assert.equal(await status(checkout({customerId: 'customer-2', sku: 'coffee', quantity: 100})), 409, 'out of stock');
  assert.equal(await status(checkout({customerId: 'customer-2', sku: 'unknown', quantity: 1})), 404, 'unknown product');
} finally {
  await shop.close();
}

// One OTLP document with one resource per service.
const spans = new Map();
for (const line of readFileSync(path.join(dir, 'traces.jsonl'), 'utf8').trim().split('\n')) {
  const [resource] = JSON.parse(line).resourceSpans;
  const service = resource.resource.attributes[0].value.stringValue;
  spans.set(service, [...(spans.get(service) || []), ...resource.scopeSpans[0].spans]);
}
rmSync(dir, {recursive: true});
const resourceSpans = [...spans].sort(([a], [b]) => a.localeCompare(b)).map(([service, list]) => ({
  resource: {attributes: [{key: 'service.name', value: {stringValue: service}}]},
  scopeSpans: [{scope: {name: 'service-shop/runtime/http.js'}, spans: list}],
}));
const out = path.join(root, '.archguard', 'otel-traces.json');
mkdirSync(path.dirname(out), {recursive: true});
writeFileSync(out, JSON.stringify({resourceSpans}, null, 1) + '\n');
console.log(`Recorded ${[...spans.values()].flat().length} spans from ${spans.size} services in ${path.relative(root, out)}`);
