import org.elasticsearch.action.admin.indices.create.CreateIndexRequest;

class VarName {
    static final String BODY = "{\"mappings\":{\"properties\":{\"a\":{\"type\":\"text\"}}}}";
    void run(String name) {
        new CreateIndexRequest(name).source(BODY, null);
    }
}
