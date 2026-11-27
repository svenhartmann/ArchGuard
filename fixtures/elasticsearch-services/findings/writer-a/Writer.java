package shop;

import co.elastic.clients.elasticsearch.ElasticsearchClient;
import co.elastic.clients.elasticsearch.core.*;

class WriterA {
    void run() {
        var a = new IndexRequest("orders-v1");
        var b = new IndexRequest("audit-log");
        var c = new IndexRequest("undeclared-ix");
        var d = new SearchRequest.Builder().index("logs-app");
    }
}
