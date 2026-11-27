import * as es from '@elastic/elasticsearch';

const INDEX = 'consts-v1';
const FIELDS = { properties: { code: { type: 'keyword' } } } as const;
const base = { number_of_shards: 1 };
const client = new es.Client({ node: 'http://localhost:9200' });

export function make(prefix: string) {
  client.indices.create({ index: INDEX, mappings: FIELDS, settings: { number_of_replicas: 0 } });
  // spread, call and template string are gaps for their attribute only
  client.indices.create({
    index: 'partial-v1',
    mappings: { properties: { id: { type: 'keyword' } } },
    settings: { ...base, number_of_replicas: 1 },
  });
  client.indices.create({ index: 'calls-v1', settings: tune(), mappings: { properties: { a: { type: `k${prefix}` } } } });
  client.indices.create({ index: prefix + '-v1', mappings: FIELDS });
}
