import assert from 'node:assert/strict';
import {Readable} from 'node:stream';
import {test} from 'node:test';
import {createClient, createServer, HttpError} from '../runtime/http.js';

function request(server, {method = 'GET', path = '/', chunks = [], headers = {}} = {}) {
  return new Promise(resolve => {
    const incoming = Readable.from(chunks);
    Object.assign(incoming, {method, url: path, headers});
    const response = {statusCode: 200, headers: {}, setHeader(name, value) { this.headers[name] = value; }, end(body) { resolve({status: this.statusCode, headers: this.headers, body: JSON.parse(body)}); }};
    server.emit('request', incoming, response);
  });
}

test('HTTP boundary parses JSON, exposes health, and rejects malformed or excessive input', async () => {
  let calls = 0;
  const server = createServer(async input => { calls++; return input.body; });
  assert.equal((await request(server, {path: '/health'})).status, 200);
  assert.equal(calls, 0);
  const valid = await request(server, {method: 'POST', path: '/checkout', chunks: ['{"quantity":2}'], headers: {'content-type': 'application/json'}});
  assert.deepEqual(valid.body, {quantity: 2});
  assert.equal(valid.headers['content-type'], 'application/json; charset=utf-8');
  assert.equal((await request(server, {method: 'POST', chunks: ['{bad'], headers: {'content-type': 'application/json'}})).status, 400);
  assert.equal((await request(server, {method: 'POST', chunks: ['a'.repeat(65537)], headers: {'content-type': 'application/json'}})).status, 413);
  assert.equal((await request(server, {method: 'POST', chunks: ['{}'], headers: {'content-type': 'text/plain'}})).status, 415);
  assert.equal(calls, 1, 'invalid request bodies never reach business handlers');
});

test('HTTP errors are explicit and unexpected implementation errors are not leaked', async () => {
  const known = createServer(async () => { throw new HttpError(409, 'stock unavailable'); });
  assert.deepEqual((await request(known)).body, {error: 'stock unavailable'});
  const broken = createServer(async () => { throw new Error('internal secret'); });
  assert.deepEqual(await request(broken), {status: 500, headers: {'content-type': 'application/json; charset=utf-8'}, body: {error: 'Internal demo service error'}});
});

test('configured HTTP client sends JSON with a bounded timeout and preserves downstream business rejection', async () => {
  const seen = [];
  const client = createClient({PAYMENTS_API: 'http://payments:8080'}, async (url, options) => {
    seen.push([String(url), options]);
    return {ok: false, status: 409, json: async () => ({error: 'risk rejected'})};
  });
  await assert.rejects(client('PAYMENTS_API', '/authorize', {method: 'POST', body: {amountCents: 150000}}), error => error.status === 409 && error.message === 'risk rejected');
  assert.equal(seen[0][0], 'http://payments:8080/authorize');
  assert.equal(seen[0][1].body, '{"amountCents":150000}');
  assert.ok(seen[0][1].signal instanceof AbortSignal);
  await assert.rejects(client('MISSING_API', '/'), error => error.status === 503);
  const failing = createClient({PAYMENTS_API: 'http://payments:8080'}, async () => { throw new Error('connection details'); });
  await assert.rejects(failing('PAYMENTS_API', '/authorize'), error => error.status === 502 && !error.message.includes('connection details'));
});
