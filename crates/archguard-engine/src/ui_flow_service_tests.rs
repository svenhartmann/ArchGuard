//! Tests of the service layers; the call model is the real javac helper (skipped without a JDK).
use archguard_core::UiFlow;
use archguard_core::{AccessState, Condition, EndpointAccess, ServiceHow, ServiceKind};

const SPRING_POM: &str = "<project/>";
const QUARKUS_POM: &str = "<project><dependencies><dependency><artifactId>quarkus-rest</artifactId></dependency></dependencies></project>";
const ENABLE: &str = "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity\nclass Conf {}\n";
const ENABLE_NONE: &str = "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\n@EnableMethodSecurity(prePostEnabled = true)\nclass Conf {}\n";

fn jdk() -> bool {
    let ok = |tool: &str| {
        std::process::Command::new(tool)
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let found = ok("java") && ok("javac");
    if !found {
        eprintln!("skipped: no JDK (java, javac) on PATH");
    }
    found
}

/// The flow of a project with `pom` and `files` (paths below `src/main/java/app`).
fn run(pom: &str, files: &[(&str, &str)]) -> Option<UiFlow> {
    if !jdk() {
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut all = vec![("pom.xml".to_string(), pom)];
    all.extend(
        files
            .iter()
            .map(|(n, t)| (format!("src/main/java/app/{n}"), *t)),
    );
    for (path, text) in all {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    Some(crate::ui_flow::analyze(dir.path()).unwrap())
}

fn ctl(handler_body: &str, extra: &str) -> String {
    format!(
        "package app;\nimport org.springframework.web.bind.annotation.*;\n@RestController\nclass Ctl {{\n  private Svc svc;\n  @GetMapping(\"/a\")\n  String a() {{ {handler_body} }}\n{extra}\n}}\n"
    )
}
fn svc(annotations: &str, method: &str) -> String {
    format!(
        "package app;\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.*;\n{annotations}\nclass Svc {{\n{method}\n}}\n"
    )
}
const PRE_B: &str = "@PreAuthorize(\"hasRole('B')\") String run() { return \"x\"; }";

fn endpoint(flow: &UiFlow) -> &EndpointAccess {
    assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
    &flow.endpoints[0]
}
fn layers(flow: &UiFlow) -> Vec<(ServiceHow, String, String)> {
    flow.service_layers_of(endpoint(flow))
        .map(|l| (l.how, l.target.clone(), l.note.clone()))
        .collect()
}
fn role(r: &str) -> AccessState {
    AccessState::Protected(Condition::Role(r.into()))
}

#[test]
fn direct_unconditional_call_is_enforced_and_anded() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            (
                "Ctl.java",
                &ctl("return svc.run();", "").replace(
                    "@GetMapping(\"/a\")",
                    "@GetMapping(\"/a\") @org.springframework.security.access.prepost.PreAuthorize(\"hasRole('A')\")",
                ),
            ),
            ("Svc.java", &svc("@Service", PRE_B)),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    assert_eq!(e.access, role("ROLE_A"));
    let l: Vec<_> = flow.service_layers_of(e).collect();
    assert_eq!(l.len(), 1, "{:#?}", flow.service_layers);
    assert_eq!(l[0].how, ServiceHow::Enforced);
    assert_eq!(l[0].kind, ServiceKind::Service);
    assert_eq!(l[0].target, "Svc.run");
    assert_eq!(l[0].state, role("ROLE_B"));
    assert_eq!(l[0].file, "src/main/java/app/Svc.java");
    assert_eq!(
        flow.effective_access(e),
        AccessState::Protected(Condition::All(vec![
            Condition::Role("ROLE_A".into()),
            Condition::Role("ROLE_B".into())
        ]))
    );
}

#[test]
fn a_call_inside_if_is_path_only_and_changes_nothing() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            (
                "Ctl.java",
                &ctl("if (svc != null) { return svc.run(); } return \"\";", ""),
            ),
            ("Svc.java", &svc("@Service", PRE_B)),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    assert_eq!(layers(&flow)[0].0, ServiceHow::PathOnly);
    assert_eq!(flow.effective_access(e), e.access);
}

#[test]
fn a_call_in_a_lambda_or_ternary_is_path_only() {
    for body in [
        "java.util.function.Supplier<String> s = () -> svc.run(); return s.get();",
        "return svc != null ? svc.run() : \"\";",
        "try { return svc.run(); } catch (RuntimeException e) { return \"\"; }",
    ] {
        let Some(flow) = run(
            SPRING_POM,
            &[
                ("Conf.java", ENABLE),
                ("Ctl.java", &ctl(body, "")),
                ("Svc.java", &svc("@Service", PRE_B)),
            ],
        ) else {
            return;
        };
        assert_eq!(layers(&flow)[0].0, ServiceHow::PathOnly, "{body}");
    }
}

#[test]
fn self_invocation_is_a_hint() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            (
                "Ctl.java",
                &ctl(
                    "return helper();",
                    "  @org.springframework.security.access.prepost.PreAuthorize(\"hasRole('B')\") String helper() { return \"\"; }",
                ),
            ),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
    assert!(l[0].2.contains("self-invocation"), "{l:?}");
    assert_eq!(flow.effective_access(e), e.access);
}

#[test]
fn a_disabled_method_security_family_is_an_ineffective_hint() {
    let secured = "@Secured(\"ROLE_B\") String run() { return \"\"; }";
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE_NONE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            (
                "Svc.java",
                &svc("@Service", secured).replace(
                    "prepost.*;",
                    "prepost.*;\nimport org.springframework.security.access.annotation.Secured;",
                ),
            ),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
    assert!(l[0].2.contains("ineffective"), "{l:?}");
    assert_eq!(flow.effective_access(e), e.access);
}

#[test]
fn post_authorize_is_a_note_not_a_check() {
    let post = "@PostAuthorize(\"hasRole('B')\") String run() { return \"\"; }";
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &svc("@Service", post)),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
    assert!(l[0].2.contains("after the call"), "{l:?}");
    assert_eq!(flow.effective_access(e), e.access);
}

const EJB: &str = "package app;\nimport jakarta.ejb.Stateless;\nimport jakarta.annotation.security.RolesAllowed;\n@Stateless\nclass Svc {\n  @RolesAllowed(\"B\") String run() { return \"\"; }\n}\n";
const CDI: &str = "package app;\nimport jakarta.enterprise.context.ApplicationScoped;\nimport jakarta.annotation.security.RolesAllowed;\n@ApplicationScoped\nclass Svc {\n  @RolesAllowed(\"B\") String run() { return \"\"; }\n}\n";
const RES: &str = "package app;\nimport jakarta.ws.rs.*;\n@Path(\"/a\")\nclass Res {\n  Svc svc;\n  @GET String a() { return svc.run(); }\n}\n";

#[test]
fn ejb_enforces_but_plain_cdi_outside_quarkus_is_a_hint() {
    let Some(ejb) = run(SPRING_POM, &[("Res.java", RES), ("Svc.java", EJB)]) else {
        return;
    };
    let l = layers(&ejb);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    assert_eq!(
        ejb.effective_access(endpoint(&ejb)),
        role("B"),
        "{:#?}",
        ejb.service_layers
    );
    let cdi = run(SPRING_POM, &[("Res.java", RES), ("Svc.java", CDI)]).unwrap();
    let l = layers(&cdi);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert_eq!(cdi.effective_access(endpoint(&cdi)), endpoint(&cdi).access);
}

#[test]
fn quarkus_cdi_beans_are_enforced() {
    let Some(flow) = run(QUARKUS_POM, &[("Res.java", RES), ("Svc.java", CDI)]) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    assert_eq!(flow.effective_access(endpoint(&flow)), role("B"));
}

#[test]
fn an_ejb_jar_descriptor_makes_ejb_annotations_a_hint_and_a_gap() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Res.java", RES),
            ("Svc.java", EJB),
            ("../../resources/META-INF/ejb-jar.xml", "<ejb-jar/>"),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert!(l[0].2.contains("ejb-jar.xml"), "{l:?}");
    assert!(!flow.complete);
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}

#[test]
fn an_imperative_check_in_the_service_is_a_hint() {
    let imperative = "String run() { return org.springframework.security.core.context.SecurityContextHolder.getContext().getAuthentication().getName(); }";
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &svc("@Service", imperative)),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
    assert!(l[0].2.contains("SecurityContextHolder"), "{l:?}");
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}

#[test]
fn a_row_filter_on_the_entity_of_a_reached_repository_is_a_data_filter() {
    let repo = "package app;\nimport org.springframework.data.jpa.repository.JpaRepository;\ninterface ItemRepo extends JpaRepository<Item, Long> {}\n";
    let item =
        "package app;\n@org.hibernate.annotations.SQLRestriction(\"tenant = 1\")\nclass Item {}\n";
    let svc = "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass Svc {\n  ItemRepo repo;\n  String run() { repo.findAll(); return \"\"; }\n}\n";
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", svc),
            ("ItemRepo.java", repo),
            ("Item.java", item),
        ],
    ) else {
        return;
    };
    let e = endpoint(&flow);
    let l: Vec<_> = flow.service_layers_of(e).collect();
    let f = l.iter().find(|l| l.how == ServiceHow::DataFilter);
    assert!(f.is_some(), "{:#?}", flow.service_layers);
    assert_eq!(f.unwrap().kind, ServiceKind::Data);
    assert_eq!(flow.effective_access(e), e.access);
}

#[test]
fn a_project_without_method_security_reads_nothing_and_stays_unchanged() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &svc("@Service", PRE_B)),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::Hint);
    assert!(l[0].2.contains("ineffective"), "{l:?}");
}

#[test]
fn an_early_return_before_the_call_makes_it_path_only_but_a_throw_does_not() {
    for (body, how) in [
        (
            "if (svc == null) return \"\"; return svc.run();",
            ServiceHow::PathOnly,
        ),
        (
            "if (svc == null) { return \"\"; } return svc.run();",
            ServiceHow::PathOnly,
        ),
        (
            "if (svc == null) throw new RuntimeException(); return svc.run();",
            ServiceHow::Enforced,
        ),
    ] {
        let Some(flow) = run(
            SPRING_POM,
            &[
                ("Conf.java", ENABLE),
                ("Ctl.java", &ctl(body, "")),
                ("Svc.java", &svc("@Service", PRE_B)),
            ],
        ) else {
            return;
        };
        assert_eq!(layers(&flow)[0].0, how, "{body}");
    }
}

#[test]
fn an_open_service_layer_never_opens_an_undeclared_endpoint() {
    let permit = "@PreAuthorize(\"permitAll()\") String run() { return \"\"; }";
    let Some(spring) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &svc("@Service", permit)),
        ],
    ) else {
        return;
    };
    let any = "@jakarta.annotation.security.PermitAll";
    let ejb = EJB.replace("@RolesAllowed(\"B\")", any);
    let cdi = CDI.replace("@RolesAllowed(\"B\")", any);
    let ejb = run(SPRING_POM, &[("Res.java", RES), ("Svc.java", &ejb)]).unwrap();
    let quarkus = run(QUARKUS_POM, &[("Res.java", RES), ("Svc.java", &cdi)]).unwrap();
    for flow in [spring, ejb, quarkus] {
        let e = endpoint(&flow);
        assert_eq!(e.access, AccessState::Undeclared);
        let effective = flow.effective_access(e);
        assert_eq!(
            effective,
            AccessState::Undeclared,
            "{:#?}",
            flow.service_layers
        );
    }
}

#[test]
fn disagreeing_implementations_are_path_only() {
    let iface = "package app;\ninterface Svc { String run(); }\n";
    let a = "package app;\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.PreAuthorize;\n@Service\nclass A implements Svc {\n  @PreAuthorize(\"hasRole('B')\") public String run() { return \"\"; }\n}\n";
    let b = "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass B implements Svc {\n  public String run() { return \"\"; }\n}\n";
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", iface),
            ("A.java", a),
            ("B.java", b),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{l:?}");
    assert_eq!(l[0].0, ServiceHow::PathOnly);
    assert!(l[0].2.contains("differ"), "{l:?}");
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}
