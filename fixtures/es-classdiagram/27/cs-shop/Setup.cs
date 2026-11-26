using Elasticsearch.Net;

class Setup
{
    void Run(IElasticLowLevelClient low, string x)
    {
        low.Indices.Create<StringResponse>("cs-orders", PostData.String(Bodies.Orders));
        low.Indices.PutTemplate<StringResponse>("cs-tpl", PostData.String(Bodies.Raw));
        low.Indices.CreateAsync<StringResponse>("cs-plain", PostData.String(Bodies.Plain));
        low.Indices.Create<StringResponse>("cs-interp", PostData.String($@"{{""mappings"":{{""properties"":{{""{x}"":{{""type"":""text""}}}}}}}}"));
        low.Indices.Create<StringResponse>("cs-fluent", PostData.Serializable(x));
        low.Tables.Create<StringResponse>("cs-lookalike", PostData.String(Bodies.Orders));
    }
}
