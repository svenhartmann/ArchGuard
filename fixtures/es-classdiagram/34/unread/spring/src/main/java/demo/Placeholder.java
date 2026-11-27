package demo;

import org.springframework.data.elasticsearch.annotations.Document;

@Document(indexName = "${app.index}")
public class Placeholder {
}
