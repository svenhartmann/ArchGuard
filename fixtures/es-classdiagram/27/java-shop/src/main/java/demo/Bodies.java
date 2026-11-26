package demo;

public class Bodies {
    public static final String INDEX = "invoices-v1";
    public static final String ORDERS = "{\"mappings\":{\"properties\":{\"title\":{\"type\":\"text\",\"analyzer\":\"folded\"},\"sku\":{\"type\":\"keyword\"}}},\"settings\":{\"analysis\":{\"analyzer\":{\"folded\":{\"type\":\"custom\",\"tokenizer\":\"standard\",\"filter\":[\"lowercase\",\"asciifolding\"]}}}}}";
    public static final String PART = "{\"mappings\":{\"properties\":{\"part\":{\"type\":\"text\"}}}}";
    public static final String BLOCK = """
        {
          "index_patterns": ["blk-*"],
          "template": { "mappings": { "properties": { "k": { "type": "keyword" } } } }
        }
        """;
    public static final String COMPOSED = "{\"mappings\":{\"properties\":{\"c\":{\"type\":\"" + "text\"}}}}";
    public static final String NOT_JSON = "{ not json";
    public static final String UNUSED = "{\"mappings\":{\"properties\":{\"unused\":{\"type\":\"text\"}}}}";
}
