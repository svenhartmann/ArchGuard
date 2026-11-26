package demo.shop;

import org.springframework.data.elasticsearch.annotations.*;

@Document(indexName = "mapped-idx")
@Setting(settingPath = "settings/mapped-settings.json")
@Mapping(mappingPath = "/mappings/mapped.json")
public class Mapped {
    @Field(type = FieldType.Text, name = "ignored")
    private String ignored;
}
