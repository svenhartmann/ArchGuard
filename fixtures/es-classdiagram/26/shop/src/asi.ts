import { Client } from '@elastic/elasticsearch'

const NAME = 'asi-v1'
const PROPS = { a: { type: 'text' } }
const client = new Client({ node: 'http://localhost:9200' })

export function make() {
  client.indices.create({ index: NAME, mappings: { properties: PROPS } })
}
