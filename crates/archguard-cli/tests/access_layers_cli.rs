//! `archguard access-layers`: a Spring fixture with a check on the service only (needs a JDK).
use std::{fs, process::Command};

fn jdk() -> bool {
    ["java", "javac"].iter().all(|t| {
        Command::new(t)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn run(args: &[&str]) -> Option<(i32, String)> {
    if !jdk() {
        eprintln!("skipped: no JDK");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        ("pom.xml", "<project/>"),
        (
            "src/main/java/app/Conf.java",
            "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity\nclass Conf {}\n",
        ),
        (
            "src/main/java/app/Ctl.java",
            "package app;\nimport org.springframework.web.bind.annotation.*;\n@RestController\nclass Ctl {\n  private Svc svc;\n  @GetMapping(\"/a\")\n  String a() { return svc.run(); }\n}\n",
        ),
        (
            "src/main/java/app/Svc.java",
            "package app;\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.*;\n@Service\nclass Svc {\n@PreAuthorize(\"hasRole('B')\") String run() { return \"x\"; }\n}\n",
        ),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .arg("access-layers")
        .arg(dir.path())
        .args(args)
        .output()
        .unwrap();
    Some((
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
    ))
}

#[test]
fn text_shows_the_service_check_and_flags_service_only() {
    let Some((code, out)) = run(&["--only", "service-guarded"]) else {
        return;
    };
    assert_eq!(code, 0);
    assert!(out.contains("GET /a  [service-only]"), "{out}");
    assert!(out.contains("Svc.run: role ROLE_B"), "{out}");
}

#[test]
fn json_carries_enforced_at() {
    let Some((_, out)) = run(&["--format", "json"]) else {
        return;
    };
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["enforced_at"], "service");
    assert_eq!(v[0]["service_guarded"], true);
}
