package demo.shop;

import java.time.Instant;
import org.springframework.data.annotation.Id;
import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Field;
import org.springframework.data.elasticsearch.annotations.FieldType;
import org.springframework.data.elasticsearch.annotations.InnerField;
import org.springframework.data.elasticsearch.annotations.MultiField;
import org.springframework.data.elasticsearch.annotations.Setting;

/** Mentioned only in a comment: @Document(indexName = "in-a-comment") */
@Document(indexName = "orders-v1")
@Setting(shards = 3, replicas = 2, refreshInterval = "5s")
public class Order {
    @Id
    private String id;

    @Field(type = FieldType.Text, analyzer = "folded", searchAnalyzer = "folded_search")
    private String title;

    @Field(name = "sku_code", type = FieldType.Keyword, normalizer = "lowercase")
    private String sku;

    @MultiField(
        mainField = @Field(type = FieldType.Text, analyzer = "folded"),
        otherFields = {@InnerField(suffix = "raw", type = FieldType.Keyword)})
    private String customerName;

    @Field(type = FieldType.Date)
    private Instant placed;

    @Field(type = FieldType.Auto)
    private String untyped;

    private String note = "@Field(type = FieldType.Text)";
}
