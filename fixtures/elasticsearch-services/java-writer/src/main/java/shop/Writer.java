package shop;

import co.elastic.clients.elasticsearch.ElasticsearchClient;
import co.elastic.clients.elasticsearch.core.*;
import java.util.List;

class Writer {
    void run(ElasticsearchClient client) {
        IndexRequest<Doc> one = IndexRequest.of(b -> b.index("orders-v1").document(doc));
        var bulk = new BulkRequest.Builder().operations(op -> op.index(i -> i.index("audit-log").document(doc)));
        var old = new IndexRequest("legacy-1");
        var all = new SearchRequest.Builder().index(List.of("orders-v1", "orders-archive"));
        var multi = new MsearchRequest.Builder().searches(s -> s.header(h -> h.index("hdr")));
        Object other = builder.index("misc");
    }
}
