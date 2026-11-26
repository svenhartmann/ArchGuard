from elasticsearch_dsl import (
    Date,
    Document,
    InnerDoc,
    Integer,
    Keyword,
    Nested,
    Object,
    Text,
    analyzer,
    char_filter,
    normalizer,
    token_filter,
    tokenizer,
)

INDEX_NAME = "orders-v1"

fold = token_filter("fold_ascii", "asciifolding", preserve_original=True)
trigram = tokenizer("tri", "ngram", min_gram=3, max_gram=3)
strip_html = char_filter("strip_html", "html_strip")
title_analyzer = analyzer(
    "title_an", tokenizer=trigram, filter=["lowercase", fold], char_filter=[strip_html]
)
lower = normalizer("lower_norm", filter=["lowercase"])


class Line(InnerDoc):
    sku = Keyword(normalizer=lower)
    qty = Integer()


class Order(Document):
    title = Text(
        analyzer=title_analyzer, search_analyzer="standard", fields={"raw": Keyword()}
    )
    placed = Date()
    customer = Object(properties={"name": Text()})
    lines = Nested(Line)

    class Index:
        name = INDEX_NAME
        settings = {"number_of_shards": 2}
        aliases = {"orders": {}}
