import { Client } from '@elastic/elasticsearch'

const client = new Client({ node: 'http://localhost:9200' })

export async function run() {
  await client.search({ index: 'products-v2', query: { match_all: {} } })
  await client.index({ index: 'products-v2', document: { a: 1 } })
  await client.msearch({ index: `tpl-index` })
  await client.get({ id: '1', index: 'single-doc' })
}
