package demo.shop;

import org.opensearch.data.annotations.Document;
import org.opensearch.data.annotations.Field;
import org.opensearch.data.annotations.FieldType;

@Document(indexName = Names.CUSTOMERS)
public class Customer {
    @Field(type = FieldType.Keyword)
    private String email;
}
