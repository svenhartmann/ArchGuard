import { Client } from '@elastic/elasticsearch';

const cache = new Map();
const client = new Client({ node: 'http://localhost:9200' });

export function other() {
  // not a client: same method names on other objects
  cache.indices.create({ index: 'cache-v1', mappings: { properties: { x: { type: 'text' } } } });
  client.create({ index: 'doc-v1', id: '1', document: { a: 1 } });
  client.indices.exists({ index: 'exists-v1' });
}
