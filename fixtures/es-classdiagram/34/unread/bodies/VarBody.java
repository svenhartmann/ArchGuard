import org.elasticsearch.action.admin.indices.create.CreateIndexRequest;

class VarBody {
    void run(String body) {
        new CreateIndexRequest("java-var-body").source(body, null);
    }
}
