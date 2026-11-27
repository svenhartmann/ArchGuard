package demo;

import org.springframework.data.elasticsearch.annotations.Document;

@Document(indexName = Names.indexName())
public class VarName {
}
