import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

export function nonAscii() {
  client.indices.create({
    index: 'umlaut-v1',
    settings: tune('größe', "ñé"),
    mappings: { properties: { title: { type: 'text' } } },
  });
}
