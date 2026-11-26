package demo.catalog;

import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Field;
import org.springframework.data.elasticsearch.annotations.FieldType;

@Document(indexName = "catalog-idx")
public class Catalog {
    @Field(type = FieldType.TokenCount, analyzer = "folded")
    private String words;

    @Field(analyzer = "folded", searchAnalyzer = "folded_search")
    private String label;

    @Field(type = FieldType.Auto)
    private String plain;
}
