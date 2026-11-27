import { Client } from '@elastic/elasticsearch'; const client = new Client({}); client.indices.create({ index: 'in-test-v1', settings: { number_of_shards: 1 } });
