import { Client } from '@elastic/elasticsearch';

const client = new Client({ node: 'http://localhost:9200' });

export function bodyVariable(req: unknown) {
  client.indices.create({ index: 'body-var-v1', body: req });
  client.indices.create({ index: 'body-call-v1', body: build() });
}
