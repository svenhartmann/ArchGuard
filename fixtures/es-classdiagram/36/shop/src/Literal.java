import org.elasticsearch.client.RestHighLevelClient;
import org.elasticsearch.action.search.SearchRequest;

class Literal {
    void find(RestHighLevelClient c) {
        c.search(new SearchRequest("docs"), null);
    }
}
