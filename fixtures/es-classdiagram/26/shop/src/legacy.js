const { Client: Es } = require('@elastic/elasticsearch');

class Admin {
  constructor() {
    this.es = new Es({ node: 'http://localhost:9200' });
  }

  run() {
    return this.es.indices.create({
      index: 'legacy-v1',
      body: {
        mappings: { properties: { name: { type: 'text' } } },
        settings: { number_of_shards: 1 },
      },
    });
  }
}
