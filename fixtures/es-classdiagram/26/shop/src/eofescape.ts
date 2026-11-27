import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

client.indices.create({ index: 'eof-v1', mappings: { properties: { title: { type: 'text' } } }, settings: f('\