package demo;

import org.springframework.data.elasticsearch.annotations.Document;

@Document(indexName = "logs-#{@clock.today()}")
public class SpEl {
}
