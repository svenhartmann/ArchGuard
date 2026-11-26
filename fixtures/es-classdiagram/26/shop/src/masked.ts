import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

// client.indices.create({ index: 'commented-v1', settings: { number_of_shards: 1 } });
/* client.indices.create({ index: 'block-v1', settings: { number_of_shards: 1 } }); */
export const DOC = "client.indices.create({ index: 'instring-v1', settings: { number_of_shards: 1 } })";
