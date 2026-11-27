//! An input that cannot be read is a gap, never a silent skip: the flow is incomplete.
//! Covers the build-file probes of `ui_flow_micronaut` (`read_build`, the discovery
//! settings) and of `ui_flow_quarkus` (`server_build`, `quarkus_build`).

use archguard_core::UiFlow;
use archguard_flow_jvm::{ui_flow_micronaut, ui_flow_quarkus};

/// `FF FE` announces UTF-16, the single byte after it is half a unit: `InvalidData`.
const BROKEN_UTF16: &[u8] = &[0xFF, 0xFE, 0x41];

const MICRONAUT: &str = "package a;\nimport io.micronaut.http.annotation.Controller;\n\
    @Controller(\"/m\") public class M { }\n";
const QUARKUS: &str = "package a;\nimport jakarta.ws.rs.GET;\nimport jakarta.ws.rs.Path;\n\
    @Path(\"/q\") public class Q { @GET public String get() { return \"\"; } }\n";

fn tree(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (rel, bytes) in files {
        std::fs::write(dir.path().join(rel), bytes).unwrap();
    }
    dir
}

fn assert_gap(flow: &UiFlow, file: &str) {
    assert!(
        !flow.complete,
        "an unreadable {file} must not leave the flow complete"
    );
    assert!(
        flow.diagnostics.iter().any(|d| d.file == file),
        "no gap names {file}: {:?}",
        flow.diagnostics
    );
}

#[test]
fn micronaut_unreadable_build_file_is_a_gap() {
    let dir = tree(&[("pom.xml", BROKEN_UTF16), ("M.java", MICRONAUT.as_bytes())]);
    assert_gap(&ui_flow_micronaut::extract(dir.path()), "pom.xml");
}

#[test]
fn micronaut_unreadable_discovery_settings_are_a_gap() {
    let dir = tree(&[("archguard.toml", b"[project]\nexclude = 1\n")]);
    assert_gap(&ui_flow_micronaut::extract(dir.path()), ".");
}

#[test]
fn quarkus_unreadable_build_file_is_a_gap() {
    let dir = tree(&[("pom.xml", BROKEN_UTF16), ("Q.java", QUARKUS.as_bytes())]);
    assert_gap(&ui_flow_quarkus::extract(dir.path()), "pom.xml");
}
