import { Client } from '@elastic/elasticsearch';

const BODY = { mappings: { properties: { title: { type: 'text' } } } };
const client = new Client({ node: 'http://localhost:9200' });

export function bodyConstant() {
  client.indices.create({ index: 'bodyconst-v1', body: BODY });
}
