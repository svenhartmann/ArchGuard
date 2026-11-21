use crate::extract_ctx::*;

#[test]
fn ancestors_walk_up_to_the_root() {
    assert_eq!(ancestors("a/b/c.cs").collect::<Vec<_>>(), ["a/b", "a", ""]);
    assert_eq!(ancestors("c.cs").collect::<Vec<_>>(), [""]);
}

#[test]
fn files_are_read_without_byte_order_mark_and_capped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.cs"), "\u{FEFF}class A {}").unwrap();
    std::fs::write(dir.path().join("big.cs"), "x".repeat(20)).unwrap();
    assert_eq!(read_text(dir.path(), "a.cs", 100).unwrap(), "class A {}");
    assert!(read_text(dir.path(), "big.cs", 10).is_err());
    let mut paths = vec!["a".to_string(), "b".into(), "c".into()];
    assert_eq!(cap(&mut paths, 2).as_deref(), Some("c"));
    assert_eq!(paths, ["a", "b"]);
    assert_eq!(cap(&mut paths, 2), None);
    let name = dir_name(dir.path(), "");
    assert_eq!(
        Some(name.as_str()),
        dir.path().file_name().and_then(|n| n.to_str())
    );
    assert_eq!(dir_name(dir.path(), "src/Web"), "Web");
}

#[test]
fn walks_honour_exclusions_and_domain_sources_skip_tests() {
    let dir = tempfile::tempdir().unwrap();
    for rel in [
        "src/A.cs",
        "node_modules/x/B.cs",
        "tests/ATests.cs",
        "bin/C.cs",
    ] {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "class X {}").unwrap();
    }
    let mut files = walk(dir.path()).unwrap().files;
    files.sort();
    assert!(files.contains(&"src/A.cs".to_string()), "{files:?}");
    assert!(
        !files.iter().any(|f| f.starts_with("node_modules")),
        "{files:?}"
    );
    let (sources, gaps) = domain_sources(dir.path(), &["bin"], |r| r.ends_with(".cs"), "C#");
    let paths: Vec<&str> = sources.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(paths, ["src/A.cs"]);
    assert!(gaps.is_empty(), "{gaps:?}");
}

#[test]
fn the_root_configuration_is_read_or_its_error_reported() {
    let dir = tempfile::tempdir().unwrap();
    with_root_config(dir.path(), |ctx| {
        assert!(ctx.config.views.access.is_empty() && ctx.config_error.is_none());
    });
    std::fs::create_dir(dir.path().join(".archguard")).unwrap();
    let file = dir.path().join(".archguard/archguard.toml");
    std::fs::write(&file, "[views.access]\nauthGuard = \"authenticated\"\n").unwrap();
    with_root_config(dir.path(), |ctx| {
        assert_eq!(ctx.config.views.access.len(), 1);
    });
    std::fs::write(&file, "[views.access]\nx = \"maybe\"\n").unwrap();
    with_root_config(dir.path(), |ctx| {
        let error = ctx.config_error.as_deref().unwrap();
        assert!(error.contains("archguard.toml"), "{error}");
        assert!(ctx.config.views.access.is_empty());
    });
}

fn arg(text: &str) -> crate::lang_ts::Item {
    crate::lang_ts::Parser { s: text, i: 0 }.value(0)
}

#[test]
fn access_condition_reads_meanings_arguments_and_fallbacks() {
    use archguard_core::{AccessMeaning as M, AccessState as S, Condition as C};
    let access: std::collections::BTreeMap<String, M> = [
        ("open", M::Open),
        ("auth", M::Authenticated),
        ("never", M::Never),
        ("admin", M::Role("admin".into())),
        ("roles", M::RoleFromArguments),
        ("pols", M::PolicyFromArguments),
        (
            "maybe",
            M::FromArgumentsOr(Box::new(M::RoleFromArguments), Box::new(M::Authenticated)),
        ),
    ]
    .map(|(k, v)| (k.to_string(), v))
    .into();
    let one = [arg("'a'")];
    let two = [arg("'a'"), arg("\"b\"")];
    let dynamic = [arg("x")];
    let cond = |name: &str, args: &[crate::lang_ts::Item]| access_condition(&access, name, args);
    assert_eq!(
        cond("unlisted", &[]),
        Ok(S::Protected(C::Custom("unlisted".into())))
    );
    assert_eq!(cond("open", &[]), Ok(S::Open));
    assert_eq!(cond("open", &one), Ok(S::Open));
    assert_ne!(cond("unlisted", &[]), Ok(S::Open));
    assert_eq!(cond("auth", &[]), Ok(S::Protected(C::Authenticated)));
    assert_eq!(cond("never", &[]), Ok(S::Protected(C::Never)));
    assert_eq!(
        cond("admin", &one),
        Ok(S::Protected(C::Role("admin".into())))
    );
    assert_eq!(cond("roles", &one), Ok(S::Protected(C::Role("a".into()))));
    assert_eq!(
        cond("pols", &two),
        Ok(S::Protected(C::Any(vec![
            C::Policy("a".into()),
            C::Policy("b".into())
        ])))
    );
    // A `$arg` form needs string literal arguments; the fallback applies without any.
    assert_eq!(cond("roles", &[]), Err("roles".into()));
    assert_eq!(cond("roles", &dynamic), Err("roles".into()));
    assert_eq!(cond("maybe", &[]), Ok(S::Protected(C::Authenticated)));
    assert_eq!(cond("maybe", &one), Ok(S::Protected(C::Role("a".into()))));
}
