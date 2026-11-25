//! The Rust/Slint readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_rust::{ui_flow_rust, ui_flow_slint};

const RS: &str = r##"
use axum::{routing::{get, post}, Router};
use leptos::*;
use leptos_router::*;

#[derive(Clone)] struct S { n: u32 }
#[component] pub fn App() -> impl IntoView {
    view! { <Router><Routes><Route path="/a" view=A/><Route path="/b/:id" view=|| view! { <B/> }/></Routes></Router> }
}
#[get("/x")] async fn x() -> &'static str { r#"é"# }
fn app() -> Router { Router::new().route("/y", get(y).post(z)).nest("/n", sub()).layer(auth) }
#[tauri::command] fn cmd(a: u32) -> String { String::new() }
fn main() { slint::include_modules!(); let ui = Main::new().unwrap(); ui.on_go(|| {}); ui.run().unwrap(); }
"##;
const SLINT: &str = r#"
import { Button } from "std-widgets.slint";
export component Main inherits Window {
    callback go(int);
    in-out property <string> name: "é";
    Button { text: "Go"; clicked => { root.go(1); } }
    if root.name == "x" : Rectangle { }
}
"#;

#[test]
fn rust_extractors_survive_broken_files() {
    let files = [
        (
            "Cargo.toml",
            "[package]\nname=\"x\"\n[dependencies]\naxum=\"0.7\"\nleptos=\"0.6\"\n",
        ),
        ("src/main.rs", RS),
        ("src/lib.rs", RS),
        ("ui/main.slint", SLINT),
    ];
    dirs::intact_tree(&files, |root| {
        dirs::assert_flow("ui_flow_rust::extract", &ui_flow_rust::extract(root));
        dirs::assert_flow("ui_flow_slint::extract", &ui_flow_slint::extract(root));
    });
    dirs::each_broken_tree(&files, |root| {
        ui_flow_rust::extract(root);
        ui_flow_slint::extract(root);
    });
}
