package demo.shop;

import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Mapping;

@Document(indexName = "missing-idx")
@Mapping(mappingPath = "mappings/absent.json")
public class Missing {
}
