use crate::clones::*;
use crate::walk::Lang;
use std::time::Instant;

/// A TypeScript function of `8 + 9 * statements + 1` tokens.
fn function(name: &str, var: &str, statements: usize) -> String {
    let mut out = format!("function {name}(a, b) {{\n");
    for i in 0..statements {
        out.push_str(&format!("  let {var}{i} = a + b * {i};\n"));
    }
    out.push_str("}\n");
    out
}

fn scan(files: &[&str], normalize: bool) -> CloneScan {
    let files: Vec<_> = files.iter().map(|s| (Lang::Ts, *s)).collect();
    find_clones(
        &files,
        &CloneOptions {
            normalize,
            ..CloneOptions::default()
        },
    )
}

#[test]
fn copied_function_of_126_tokens_is_one_group_with_both_locations() {
    let body = function("f", "x", 13);
    let a = format!("const head = 1\n{body}");
    let b = format!("export const other = [1, 2];\n\n{body}const tail = 2;\n");
    let result = scan(&[&a, &b], false);
    assert!(result.truncated.is_none());
    assert_eq!(result.groups.len(), 1, "{:?}", result.groups);
    let group = &result.groups[0];
    assert_eq!(group.tokens, 126);
    assert_eq!(
        group.occurrences,
        vec![
            CloneLocation {
                file: 0,
                line: 2,
                end_line: 16,
                column: 1,
                end_column: 1
            },
            CloneLocation {
                file: 1,
                line: 3,
                end_line: 17,
                column: 1,
                end_column: 1
            },
        ]
    );
}

#[test]
fn comments_and_layout_do_not_break_a_copy() {
    let a = function("f", "x", 13);
    let b = a
        .replace("\n  ", "\n      // reformatted\n ")
        .replace(" = ", "=");
    assert_eq!(scan(&[&a, &b], false).groups.len(), 1);
}

#[test]
fn renamed_identifiers_match_only_in_normalized_mode() {
    let a = function("f", "x", 13);
    let b = function("g", "y", 13).replace("a + b", "p + q");
    assert!(scan(&[&a, &b], false).groups.is_empty());
    let normalized = scan(&[&a, &b], true);
    assert_eq!(normalized.groups.len(), 1);
    assert_eq!(normalized.groups[0].occurrences.len(), 2);
}

#[test]
fn normalization_keeps_keywords_apart() {
    let a = function("f", "x", 13);
    let b = a.replace("let ", "const ");
    assert!(scan(&[&a, &b], true).groups.is_empty());
}

#[test]
fn copy_below_the_threshold_is_not_reported() {
    let a = function("f", "x", 8); // 8 + 72 + 1 = 81 tokens
    let b = format!("const z = 0;\n{a}");
    assert!(scan(&[&a, &b], false).groups.is_empty());
    let lower = find_clones(
        &[(Lang::Ts, a.as_str()), (Lang::Ts, b.as_str())],
        &CloneOptions {
            min_tokens: 50,
            ..CloneOptions::default()
        },
    );
    assert_eq!(lower.groups.len(), 1);
    assert_eq!(lower.groups[0].tokens, 81);
}

#[test]
fn three_copies_form_one_group() {
    let body = function("f", "x", 13);
    let result = scan(&[&body, &body, &body], false);
    assert_eq!(result.groups.len(), 1);
    assert_eq!(result.groups[0].occurrences.len(), 3);
}

#[test]
fn repeated_copies_inside_one_file_never_overlap() {
    let body = function("f", "x", 13);
    let file = format!("{body}const sep = 1;\n{body}");
    let result = scan(&[&file], false);
    assert_eq!(result.groups.len(), 1);
    let o = &result.groups[0].occurrences;
    assert_eq!((o[0].line, o[0].end_line), (1, 15));
    assert_eq!((o[1].line, o[1].end_line), (17, 31));
    let repetitive = "x = 1;\n".repeat(400);
    for group in scan(&[&repetitive], false).groups {
        for pair in group.occurrences.windows(2) {
            assert!(pair[0].end_line < pair[1].line, "{group:?}");
        }
    }
}

#[test]
fn token_budget_truncation_is_reported_not_hidden() {
    let body = function("f", "x", 13);
    let result = find_clones(
        &[(Lang::Ts, body.as_str()), (Lang::Ts, body.as_str())],
        &CloneOptions {
            max_tokens: 200,
            ..CloneOptions::default()
        },
    );
    assert_eq!(result.files_scanned, 1);
    assert!(
        result
            .truncated
            .as_deref()
            .unwrap()
            .contains("token budget")
    );
    let expired = find_clones(
        &[(Lang::Ts, body.as_str()), (Lang::Ts, body.as_str())],
        &CloneOptions {
            deadline: Some(Instant::now()),
            ..CloneOptions::default()
        },
    );
    assert!(expired.truncated.as_deref().unwrap().contains("time limit"));
}

#[test]
fn other_languages_use_their_own_comment_rules() {
    let rust: String = (0..30)
        .map(|i| format!("    let v{i} = w.get({i}); // note {i}\n"))
        .collect();
    let a = format!("fn f(w: &W) {{\n{rust}}}\n");
    let b = a.replace("// note", "/* other */ //");
    let result = find_clones(
        &[(Lang::Rust, a.as_str()), (Lang::Rust, b.as_str())],
        &CloneOptions::default(),
    );
    assert_eq!(result.groups.len(), 1);
    let python: String = (0..30)
        .map(|i| format!("    v{i} = w.get({i})  # note {i}\n"))
        .collect();
    let p = format!("def f(w):\n{python}");
    let q = p.replace("# note", "# changed");
    let result = find_clones(
        &[(Lang::Py, p.as_str()), (Lang::Py, q.as_str())],
        &CloneOptions::default(),
    );
    assert_eq!(result.groups.len(), 1);
}
