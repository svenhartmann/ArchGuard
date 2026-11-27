import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });
let PROBE_CONST = { mappings: { properties: { title: { type: 'text' } } } };

export function constBody() {
  client.indices.create({ index: 'p1', body: PROBE_CONST });
}
