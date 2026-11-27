import { Client } from '@elastic/elasticsearch';

const settings = { number_of_shards: 9 };
const client = new Client({ node: 'http://localhost:9200' });

export async function setup() {
  const settings = await loadSettings();
  client.indices.create({ index: 'shadow-v1', settings });
}

export function viaParameter(settings: object) {
  client.indices.create({ index: 'shadow-param-v1', settings });
}
