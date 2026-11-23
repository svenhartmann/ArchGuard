using Nest;

class Reader
{
    void Run(IElasticClient client)
    {
        var a = client.Search<Doc>(s => s.Index("orders-v1"));
        client.IndexMany(docs, "audit-log");
        client.Index(doc, i => i.Index("reader-out"));
        var r = new SearchRequest<Doc>(Indices.Index("stats"));
        client.Get<Doc>(1, g => g.Index(new IndexName("named")));
        var settings = new ConnectionSettings(uri).DefaultIndex("default-ix");
    }
}
