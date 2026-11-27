import org.elasticsearch.client.RestHighLevelClient;
import org.elasticsearch.action.search.SearchRequest;

class Builder {
    void find(RestHighLevelClient c) {
        c.search(new SearchRequestBuilder("builder-docs"), null);
    }
}
