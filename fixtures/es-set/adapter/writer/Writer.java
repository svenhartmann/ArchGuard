package shop;

import co.elastic.clients.elasticsearch.core.*;

class Writer {
    void run() {
        var a = new IndexRequest("orders");
        var b = new IndexRequest("audit");
    }
}
