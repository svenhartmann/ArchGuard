using Elasticsearch.Net;

static class Bodies
{
    public const string Orders = @"{""mappings"":{""properties"":{""name"":{""type"":""text""}}}}";
    public const string Raw = """
        {"index_patterns": ["cs-*"], "template": {"settings": {"number_of_shards": 1}}}
        """;
    public const string Plain = "{\"mappings\":{\"properties\":{\"plain\":{\"type\":\"keyword\"}}}}";
}
