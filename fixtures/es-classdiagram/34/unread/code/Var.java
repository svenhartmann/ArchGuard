import org.elasticsearch.client.RestHighLevelClient;
import org.elasticsearch.action.index.IndexRequest;

class Var {
    void put(RestHighLevelClient c, String name) {
        c.index(new IndexRequest(name), null);
    }
}
