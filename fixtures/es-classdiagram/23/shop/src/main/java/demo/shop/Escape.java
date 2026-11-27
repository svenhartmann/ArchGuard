package demo.shop;

import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Mapping;

@Document(indexName = "escape-idx")
@Mapping(mappingPath = "../../../secret.json")
public class Escape {
}
