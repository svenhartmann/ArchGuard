//! The C-like reader takes foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_clike::decl::{self, Dialect};
use archguard_flow_clike::{handlers, lex, source::Source};
use variants::text_variants;

const SEED: &str = r#"
package a.b;
import x.Y;
@RestController
@RequestMapping(value = {"/a", "/b"}, method = GET)
public class Foo<T> extends Bar implements Baz {
    @GetMapping("/a/{id}") public String m(@PathVariable Long id, int[] xs) { return "é\"x" + 'c'; }
    [HttpPost("p")] public async Task<IActionResult> N(int a) => Ok();
    fun k(a: Int): String = "x"
}
"#;

#[test]
fn clike_survives_broken_text() {
    for t in text_variants(SEED) {
        lex::strip_comments(&t);
        lex::split_top(&t);
        lex::split_by(&t, b',');
        lex::named_part(&t);
        lex::named(&t, "value");
        lex::positional(&t);
        lex::literals(&t);
        lex::words(&t, "class").count();
        decl::simple(&t);
        decl::directive_attrs(&t);
        handlers::sends_in(&t, (0, t.len()));
        handlers::simple_type(&t);
        let b = t.as_bytes();
        for i in 0..=b.len() {
            lex::skip_ws(b, i);
            lex::skip_ws_back(b, i);
            lex::ident_before(b, i);
        }
        for dialect in [Dialect::Java, Dialect::Kotlin, Dialect::CSharp] {
            let src = Source::new("A.java".into(), &t);
            for d in decl::decls(&src, dialect) {
                decl::class_header(&src, &d);
            }
            decl::each_method(&src, dialect, |_, _| {});
            handlers::declares(&src, "Foo");
            handlers::definitions(&src, "m").count();
            handlers::enclosing_class(&src, t.len() / 2);
            handlers::of_method(&src, "Foo", "m", t.len() / 3);
            for i in (0..t.len()).step_by(7) {
                src.line(i);
                source_statement_end(&src, i);
            }
        }
    }
}

fn source_statement_end(src: &Source, at: usize) {
    if at <= src.text.len() {
        archguard_flow_clike::source::statement_end(src, at);
    }
}
