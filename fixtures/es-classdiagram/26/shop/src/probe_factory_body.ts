import { Client } from '@elastic/elasticsearch';

const PROBE_FACTORY = { mappings: { properties: { title: { type: 'text' } } } };

export function factoryBody() {
  getClient().indices.create({ index: 'p2', body: PROBE_FACTORY });
}
