using Elastic.Clients.Elasticsearch;

class Names {
    void Find(ElasticsearchClient client, string prefix) {
        client.Search<object>(s => s.Index(prefix + "-docs"));
    }
}
