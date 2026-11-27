import { Client } from '@elastic/elasticsearch';

export const INDEX = 'exp-v1';
export const SETTINGS = { number_of_shards: 2 };
const client = new Client({ node: 'http://localhost:9200' });

client.indices.create({ index: INDEX, settings: SETTINGS });
