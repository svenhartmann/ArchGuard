import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

export async function setup(name: string) {
  await client.indices.create({ index: name, mappings: { properties: { a: { type: 'text' } } } });
}
