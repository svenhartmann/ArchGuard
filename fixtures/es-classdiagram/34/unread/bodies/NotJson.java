import org.elasticsearch.action.admin.indices.create.CreateIndexRequest;

class NotJson {
    static final String BODY = "{ not json";
    void run() {
        new CreateIndexRequest("java-not-json").source(BODY, null);
    }
}
