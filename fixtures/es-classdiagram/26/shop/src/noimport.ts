import { Client } from './own-client';

const client = new Client();
client.indices.create({ index: 'foreign-v1', mappings: { properties: { x: { type: 'text' } } } });
