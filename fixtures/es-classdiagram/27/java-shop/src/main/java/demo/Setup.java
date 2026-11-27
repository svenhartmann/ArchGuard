package demo;

import org.elasticsearch.client.indices.CreateIndexRequest;

class Setup {
    void run(RestHighLevelClient c, String prefix) {
        CreateIndexRequest orders = new CreateIndexRequest("orders-v1");
        orders.source(Bodies.ORDERS, XContentType.JSON);
        c.indices().create(new CreateIndexRequest(Bodies.INDEX).source(Bodies.PART, XContentType.JSON), DEFAULT);
        c.indices().putTemplate(new PutIndexTemplateRequest("blk-tpl").source(Bodies.BLOCK, XContentType.JSON), DEFAULT);
        c.indices().create(new CreateIndexRequest("inline-v1").source("{\"mappings\":{\"properties\":{\"i\":{\"type\":\"keyword\"}}}}", XContentType.JSON), DEFAULT);
        c.indices().create(new CreateIndexRequest("composed-v1").source(Bodies.COMPOSED, XContentType.JSON), DEFAULT);
        c.indices().create(new CreateIndexRequest("broken-v1").source(Bodies.NOT_JSON, XContentType.JSON), DEFAULT);
        c.indices().create(new CreateIndexRequest(prefix + "-v1").source(Bodies.PART, XContentType.JSON), DEFAULT);
        c.indices().create(new CreateTableRequest("lookalike-tbl").source(Bodies.ORDERS, XContentType.JSON), DEFAULT);
        client.indices().create(b -> b.index("modern-v1").withJson(new StringReader(Bodies.PART)));
        client.tables().create(b -> b.index("lookalike-modern").withJson(new StringReader(Bodies.PART)));
    }
}
