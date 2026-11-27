package demo.shop;

import org.opensearch.data.client.orhlc.OpenSearchRestTemplate;
import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Field;
import org.springframework.data.elasticsearch.annotations.FieldType;

@Document(indexName = "archive-idx")
public class Archive {
    @Field(type = FieldType.Keyword)
    private String ref;
}
