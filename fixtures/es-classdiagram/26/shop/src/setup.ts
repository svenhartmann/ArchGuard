import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

export async function setup() {
  await client.indices.create({
    index: 'orders-v1',
    mappings: {
      properties: {
        title: { type: 'text', analyzer: 'standard' }, // comment
        sku: { type: 'keyword' },
      },
    },
    settings: { number_of_shards: 2, 'index.refresh_interval': '5s' },
  });
  await client.indices.putIndexTemplate({
    name: 'logs-tpl',
    index_patterns: ['logs-*'],
    composed_of: ['logs-base'],
    template: { settings: { number_of_replicas: 1 } },
  });
  await client.cluster.putComponentTemplate({
    name: 'logs-base',
    template: { mappings: { properties: { ts: { type: 'date' } } } },
  });
  await client.ilm.putLifecycle({
    name: 'logs-ilm',
    policy: { phases: { hot: { actions: {} } } },
  });
  await client.ingest.putPipeline({
    id: 'logs-pipe',
    processors: [{ set: { field: 'a', value: 'b' } }],
  });
  await client.indices.putMapping({
    index: 'late-v1',
    properties: { extra: { type: 'keyword' } },
  });
}
