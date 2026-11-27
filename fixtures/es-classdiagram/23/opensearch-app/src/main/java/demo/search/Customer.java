package demo.search;

import demo.shop.Names;
import org.springframework.data.elasticsearch.annotations.Document;
import org.springframework.data.elasticsearch.annotations.Field;
import org.springframework.data.elasticsearch.annotations.FieldType;

@Document(indexName = Names.CUSTOMERS)
public class Customer {
    @Field(type = FieldType.Keyword)
    private String email;
}
