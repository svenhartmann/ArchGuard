package demo.shop

import org.springframework.data.elasticsearch.annotations.Document
import org.springframework.data.elasticsearch.annotations.Field
import org.springframework.data.elasticsearch.annotations.FieldType

@Document(indexName = "notes-idx")
class Note {
    @Field(type = FieldType.Text, analyzer = "plain")
    var body: String = ""
}
