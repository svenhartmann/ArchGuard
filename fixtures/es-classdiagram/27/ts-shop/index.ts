import { Client } from '@opensearch-project/opensearch';

const ORDERS = '{"mappings":{"properties":{"title":{"type":"text"}}}}';
const TPL = `{"index_patterns":["ts-*"],"template":{"settings":{"number_of_shards":1}}}`;
const TPL_X = `{"index_patterns":["${prefix}-*"]}`;
const PROPS = '{"properties":{"added":{"type":"keyword"}}}';

async function run(client: Client) {
  await client.indices.create({ index: 'ts-orders', body: ORDERS });
  await client.indices.putIndexTemplate({ name: 'ts-tpl', body: TPL });
  await client.indices.putMapping({ index: 'ts-map', body: PROPS });
  await client.indices.create({ index: 'ts-x', body: TPL_X });
  await client.indices.create({ index: 'ts-short', body });
  await client.queue.create({ index: 'ts-lookalike', body: ORDERS });
}
