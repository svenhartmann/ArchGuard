package demo.shop;

import org.springframework.data.mongodb.core.mapping.Document;

@Document(indexName = "not-elasticsearch")
public class Lookalike {
}
