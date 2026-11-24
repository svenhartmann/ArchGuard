package shop;

import co.elastic.clients.elasticsearch.ElasticsearchClient;
import co.elastic.clients.elasticsearch.core.*;

class WriterB {
    void run() {
        var a = new IndexRequest("orders");
        var b = new SearchRequest.Builder().index("audit-log");
        var c = new SearchRequest.Builder().index("also-undeclared");
    }
}
