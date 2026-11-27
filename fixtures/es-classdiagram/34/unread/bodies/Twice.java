import org.elasticsearch.action.admin.indices.create.CreateIndexRequest;

class Twice {
    static final String BODY = "{\"mappings\":{}}";
    static final String BODY = "{\"settings\":{}}";
    void run() {
        new CreateIndexRequest("java-twice").source(BODY, null);
    }
}
