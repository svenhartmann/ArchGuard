import { Client } from '@opensearch-project/opensearch';

const client = new Client({ node: 'http://localhost:9200' });
client.indices.create({ index: 'os-v1', body: { settings: { index: { number_of_shards: 3 } } } });
