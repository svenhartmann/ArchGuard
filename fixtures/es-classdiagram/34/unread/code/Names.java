import org.elasticsearch.client.RestHighLevelClient;
import org.elasticsearch.action.search.SearchRequest;

class Names {
    void find(RestHighLevelClient c, String prefix) {
        c.search(new SearchRequest(prefix + "-docs"), null);
    }
}
