import { Client } from '@elastic/elasticsearch';

const PROBE_PARAM = { mappings: { properties: { title: { type: 'text' } } } };

export function make(es: Client) {
  return es.indices.create({ index: 'p3', body: PROBE_PARAM });
}
