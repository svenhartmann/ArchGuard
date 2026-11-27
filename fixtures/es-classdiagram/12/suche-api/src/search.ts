import { Client } from '@elastic/elasticsearch'

const client = new Client({ node: 'http://localhost:9200' })

export async function run() {
  await client.search({ index: 'produkte', query: { match_all: {} } })
}
