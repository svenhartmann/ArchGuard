use archguard_engine::{Level, analyze, analyze_cached, analyze_effective, load_config};

#[test]
fn malformed_declared_config_has_field_context() {
    for (content, field) in [
        ("[rules]\ndeny_cyles = 'off'", "deny_cyles"),
        ("[rules]\ndeny_cycles = 'files'", "deny_cycles"),
        ("[rules]\nmax_acd = nan", "rules.max_acd"),
        ("[rules]\nmax_acd = inf", "rules.max_acd"),
        ("[rules]\nmax_acd = -1", "rules.max_acd"),
        (
            "[rules]\nmax_instability_core = 1.1",
            "rules.max_instability_core",
        ),
        ("[rules]\nmax_scc_size = -1", "max_scc_size"),
        ("[project]\nexclude = ['[']", "project.exclude"),
        ("[project]\nlanguages = ['brainfuck']", "project.languages"),
        (
            "[[layers]]\nname = 'a'\ncontains = ['[']",
            "layers[0].contains",
        ),
        (
            "[[layers]]\nname = 'a'\n[[layers]]\nname = 'a'",
            "layers[1].name",
        ),
        (
            "derive_layers = false\n[[layers]]\nname = 'a'\nmay_depend_on = ['missing']",
            "may_depend_on",
        ),
        ("[store]\nsnyc_on_check = true", "snyc_on_check"),
        ("[projects]\nsource_roots = ['src']", "projects"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("archguard.toml"), content).unwrap();
        let error = load_config(None, dir.path()).unwrap_err().to_string();
        assert!(error.contains(field), "{content}: {error}");
        assert!(error.contains("archguard.toml"), "{error}");
    }
}

#[test]
fn valid_partial_configs_and_programmatic_inputs_are_validated() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("archguard.toml"), "[rules]\nmax_acd = 2.5").unwrap();
    let mut loaded = load_config(None, dir.path()).unwrap();
    analyze_effective(dir.path(), &loaded, Level::Module).unwrap();
    loaded.config.rules.max_acd = f32::NAN;
    assert!(analyze_effective(dir.path(), &loaded, Level::Module).is_err());
    assert!(analyze(dir.path(), &loaded.config, Level::Module).is_err());
    assert!(analyze_cached(dir.path(), &loaded.config, Level::Module).is_err());
    assert!(!dir.path().join(".archguard").exists());
}

#[test]
fn unknown_target_is_rejected_after_derivation_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("archguard.toml"),
        "[[layers]]\nname = 'a'\nmay_depend_on = ['missing']",
    )
    .unwrap();
    let loaded = load_config(None, dir.path()).unwrap();
    let error = analyze_effective(dir.path(), &loaded, Level::Module)
        .err()
        .unwrap();
    assert!(error.to_string().contains("may_depend_on"));
}
