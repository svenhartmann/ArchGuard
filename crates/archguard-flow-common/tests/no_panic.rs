//! The shared scanners take foreign source: broken input must never panic.
#[path = "support/variants.rs"]
mod variants;

use archguard_flow_common::{
    lang_csharp, lang_java, lang_markup, lang_php, lang_python, lang_rust, lang_text, lang_ts_decl,
};
use variants::text_variants;

const SEED: &str = r##"
// héllo ünï
using System; /* block */ #region x
[HttpGet("a/{id}")] public class Foo { string s = @"q""x"; char c = '\''; }
@RestController class A { @GetMapping("/a") void m(String s) { "t\"x"; } }
#[get("/x")] fn handler(a: u32) -> &'static str { r#"raw"# }
import os, sys
from a.b import c as d
@app.route("/p", methods=["GET"])
def view(request, *args, **kw):
    """doc""" ; x = f(1, k='v')
<?php Route::get('/p', [C::class, 'm']); $x = "a{$b}"; ?>
import { Component } from '@angular/core'; export const r = [{ path: 'a', component: A }];
<div class="a" :b="c" @click="d()"><Foo bar={baz} /><!-- c --></div>
"##;

#[test]
fn scanners_survive_broken_text() {
    for t in text_variants(SEED) {
        lang_csharp::mask(&t);
        lang_csharp::code(&t);
        lang_csharp::literal(&t);
        lang_csharp::balanced(&t, ')');
        lang_csharp::split_args(&t);
        lang_csharp::ident_start(&t);
        lang_java::mask(&t);
        lang_rust::mask(&t);
        lang_rust::code(&t);
        lang_rust::literal_value(&t);
        lang_rust::string_at(&t, t.len() / 2);
        lang_rust::fn_items(&lang_rust::mask(&t));
        lang_rust::macro_bodies(&lang_rust::mask(&t), "x");
        lang_php::mask(&t);
        lang_php::code(&t);
        lang_php::literal(&t);
        let m = lang_python::mask(&t);
        lang_python::string_value(&t);
        lang_python::statements(&m);
        lang_python::code_statements(&t, &m);
        lang_python::defs(&m);
        lang_python::imports(&m);
        lang_markup::all_tags(&t);
        lang_markup::blocks(&t, &["div"]);
        lang_markup::blank(&t, "<!--", "-->");
        lang_markup::strip_tags(&t);
        lang_markup::attributes(&t);
        lang_ts_decl::mask(&t, true);
        lang_ts_decl::imports(&t);
        lang_ts_decl::bindings(&t, "use");
        lang_ts_decl::declared_value(&t, "r");
        lang_text::split_top(&t, ",");
        lang_text::find_top(&t, "x");
        for i in 0..t.len() {
            lang_text::matching(t.as_bytes(), i);
            lang_text::matching_back(t.as_bytes(), i);
        }
    }
}
