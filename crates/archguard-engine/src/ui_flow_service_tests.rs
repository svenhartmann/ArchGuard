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
            (
                "../../resources/META-INF/ejb-jar.xml",
                &jar(
                    "<method-permission><role-name>X</role-name><method><ejb-name>Svc</ejb-name><method-name>run</method-name><method-params/></method></method-permission>",
                ),
            ),
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

const ENABLE_ASPECTJ: &str = "package app;\nimport org.springframework.security.config.annotation.method.configuration.EnableMethodSecurity;\nimport org.springframework.context.annotation.AdviceMode;\n@EnableMethodSecurity(mode = AdviceMode.ASPECTJ)\nclass Conf {}\n";
const PRE_B_HELPER: &str = "  @org.springframework.security.access.prepost.PreAuthorize(\"hasRole('B')\") String helper() { return \"\"; }";

#[test]
fn aspectj_mode_enforces_self_invocation() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE_ASPECTJ),
            ("Ctl.java", &ctl("return helper();", PRE_B_HELPER)),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    assert_eq!(flow.effective_access(endpoint(&flow)), role("ROLE_B"));
}

#[test]
fn aspectj_mode_enforces_a_private_method() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE_ASPECTJ),
            (
                "Ctl.java",
                &ctl(
                    "return helper();",
                    &PRE_B_HELPER.replace("String helper", "private String helper"),
                ),
            ),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
}

#[test]
fn an_unreadable_advice_mode_keeps_the_hint_and_leaves_a_gap() {
    let conf = ENABLE_ASPECTJ.replace("AdviceMode.ASPECTJ", "Modes.CHOSEN");
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", &conf),
            ("Ctl.java", &ctl("return helper();", PRE_B_HELPER)),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert!(!flow.complete);
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}

fn jar(body: &str) -> String {
    format!("<ejb-jar><assembly-descriptor>{body}</assembly-descriptor></ejb-jar>")
}
fn permission(roles: &str, method: &str) -> String {
    format!(
        "<method-permission>{roles}<method><ejb-name>Svc</ejb-name><method-name>{method}</method-name></method></method-permission>"
    )
}
fn with_jar(xml: &str) -> Option<UiFlow> {
    let plain = EJB
        .replace("  @RolesAllowed(\"B\") ", "  ")
        .replace("import jakarta.annotation.security.RolesAllowed;\n", "");
    run(
        SPRING_POM,
        &[
            ("Res.java", RES),
            ("Svc.java", &plain),
            ("../../resources/META-INF/ejb-jar.xml", xml),
        ],
    )
}

#[test]
fn an_ejb_jar_method_permission_protects_like_the_annotation() {
    let xml = jar(&permission("<role-name>B</role-name>", "run"));
    let Some(flow) = with_jar(&xml) else { return };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    assert_eq!(flow.effective_access(endpoint(&flow)), role("B"));
    assert!(flow.complete, "{:#?}", flow.diagnostics);
}

// Jakarta EJB 4.0 section 11.3.2.2: the permissions of all matching method-permission
// elements are united; `unchecked` among them permits every role. (An earlier version
// of this test asserted that a named method beats `*`; that rule exists only for transactions.)
#[test]
fn an_ejb_jar_unites_the_matching_permissions_of_wildcard_and_named_method() {
    let xml = jar(&format!(
        "{}{}",
        permission("<role-name>W</role-name>", "*"),
        permission("<role-name>B</role-name>", "run")
    ));
    let Some(flow) = with_jar(&xml) else { return };
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        AccessState::Protected(Condition::Any(vec![
            Condition::Role("W".into()),
            Condition::Role("B".into())
        ]))
    );
    let xml = jar(&permission("<role-name>W</role-name>", "*"));
    let flow = with_jar(&xml).unwrap();
    assert_eq!(flow.effective_access(endpoint(&flow)), role("W"));
}

#[test]
fn an_unchecked_wildcard_permits_every_role_even_next_to_a_named_role() {
    let xml = jar(&format!(
        "{}{}",
        permission("<unchecked/>", "*"),
        permission("<role-name>B</role-name>", "run")
    ));
    let Some(flow) = with_jar(&xml) else { return };
    let l: Vec<_> = flow.service_layers_of(endpoint(&flow)).collect();
    assert_eq!(l[0].state, AccessState::Open, "{l:?}");
}

#[test]
fn an_ejb_jar_exclude_list_denies() {
    let xml = jar(
        "<exclude-list><method><ejb-name>Svc</ejb-name><method-name>run</method-name></method></exclude-list>",
    );
    let Some(flow) = with_jar(&xml) else { return };
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        AccessState::Protected(Condition::Never)
    );
}

#[test]
fn an_ejb_jar_entry_with_an_unreadable_part_stays_a_hint_and_a_gap() {
    let xml = jar(
        "<method-permission><role-name>B</role-name><method><ejb-name>Svc</ejb-name><method-name>run</method-name><method-params><method-param>int</method-param></method-params></method></method-permission>",
    );
    let Some(flow) = with_jar(&xml) else { return };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert!(!flow.complete);
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
    let broken = with_jar("<ejb-jar><assembly-descriptor>").unwrap();
    assert!(!broken.complete);
}

#[test]
fn a_service_call_whose_class_declaration_cannot_be_read_leaves_a_gap() {
    // The call model sees the enum `Svc`; the reader indexes classes and interfaces only.
    let rec = svc("", PRE_B).replace("class Svc {", "enum Svc { ONE;");
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            (
                "Ctl.java",
                &ctl("return svc.run();", "  void other() {} // PostAuthorize"),
            ),
            ("Svc.java", &rec),
        ],
    ) else {
        return;
    };
    let gap = (flow.diagnostics.iter()).any(|d| d.message.contains("Svc.run"));
    assert!(gap && !flow.complete, "{:#?}", flow.diagnostics);
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}

fn aspectj_conf(extra: &str) -> String {
    ENABLE_ASPECTJ.replace("mode = ", &format!("{extra}mode = "))
}
fn secured_svc(class_ann: &str, method: &str) -> String {
    svc(class_ann, method).replace(
        "prepost.*;",
        "prepost.*;\nimport org.springframework.security.access.annotation.Secured;",
    )
}

#[test]
fn aspectj_does_not_weave_a_class_level_pre_authorize() {
    let class_level = svc(
        "@Service @PreAuthorize(\"hasRole('B')\")",
        "String run() { return \"\"; }",
    );
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE_ASPECTJ),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &class_level),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert_eq!(
        flow.effective_access(endpoint(&flow)),
        endpoint(&flow).access
    );
}

#[test]
fn aspectj_weaves_a_class_level_secured_on_public_methods_only() {
    let conf = aspectj_conf("securedEnabled = true, ");
    let public = secured_svc(
        "@Service @Secured(\"ROLE_B\")",
        "public String run() { return \"\"; }",
    );
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", &conf),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &public),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    let package_private = secured_svc(
        "@Service @Secured(\"ROLE_B\")",
        "String run() { return \"\"; }",
    );
    let flow = run(
        SPRING_POM,
        &[
            ("Conf.java", &conf),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &package_private),
        ],
    )
    .unwrap();
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
}

#[test]
fn an_ejb_jar_applies_only_to_the_beans_of_its_own_module() {
    const ORDER: &str = "package app;\nimport jakarta.ejb.Stateless;\n@Stateless\nclass OrderBean {\n  String run() { return \"\"; }\n}\n";
    const RESOURCE: &str = "package app;\nimport jakarta.ws.rs.*;\n@Path(\"/a\")\nclass Res {\n  OrderBean bean;\n  @GET String a() { return bean.run(); }\n}\n";
    let xml = jar(&permission("<role-name>B</role-name>", "run").replace("Svc", "OrderBean"));
    let m = "../../../../";
    let files = [
        (format!("{m}a/pom.xml"), SPRING_POM),
        (format!("{m}a/src/main/java/app/Res.java"), RESOURCE),
        (format!("{m}a/src/main/java/app/OrderBean.java"), ORDER),
        (
            format!("{m}a/src/main/resources/META-INF/ejb-jar.xml"),
            xml.as_str(),
        ),
        (format!("{m}b/pom.xml"), SPRING_POM),
        (format!("{m}b/src/main/java/app/Res.java"), RESOURCE),
        (format!("{m}b/src/main/java/app/OrderBean.java"), ORDER),
    ];
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    let Some(flow) = run(SPRING_POM, &borrowed) else {
        return;
    };
    let of = |dir: &str| {
        flow.endpoints
            .iter()
            .find(|e| e.file.starts_with(dir))
            .unwrap()
    };
    assert_eq!(
        flow.effective_access(of("a/")),
        role("B"),
        "{:#?}",
        flow.service_layers
    );
    assert_eq!(
        flow.effective_access(of("b/")),
        of("b/").access,
        "{:#?}",
        flow.service_layers
    );
}

#[test]
fn aspectj_may_not_weave_an_interface_method_annotation() {
    let api = "package app;\nimport org.springframework.security.access.prepost.*;\ninterface Api {\n  @PreAuthorize(\"hasRole('B')\") default String run() { return \"x\"; }\n  @PreAuthorize(\"hasRole('B')\") String other();\n}\n";
    let svc = "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass Svc implements Api {\n  public String other() { return \"\"; }\n}\n";
    for call in ["svc.run()", "svc.other()"] {
        let ctl = ctl(&format!("return {call};"), "").replace("private Svc svc", "private Api svc");
        let Some(flow) = run(
            SPRING_POM,
            &[
                ("Conf.java", ENABLE_ASPECTJ),
                ("Ctl.java", &ctl),
                ("Api.java", api),
                ("Svc.java", svc),
            ],
        ) else {
            return;
        };
        let l = layers(&flow);
        assert!(
            l.iter().all(|l| l.0 != ServiceHow::Enforced),
            "{call}: {l:?}"
        );
        if call == "svc.run()" {
            assert_eq!(l[0].0, ServiceHow::Hint, "{call}: {l:?}");
            assert!(
                (flow.diagnostics.iter()).any(|d| d.message.contains("interface")),
                "{:#?}",
                flow.diagnostics
            );
        }
    }
}

#[test]
fn modifiers_on_the_line_before_the_method_still_count() {
    let split = svc(
        "@Service",
        "@PreAuthorize(\"hasRole('B')\")\n  final String run() { return \"x\"; }",
    );
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &split),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
}

#[test]
fn annotations_before_a_modifier_only_line_are_read() {
    let split = svc(
        "@Service",
        "@PreAuthorize(\"hasRole('B')\") final\n  String run() { return \"x\"; }",
    );
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &split),
        ],
    ) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
}

#[test]
fn a_class_missing_from_the_call_model_leaves_a_gap() {
    super::ui_flow_service::DROPPED.with(|d| *d.borrow_mut() = vec!["app.Svc".into()]);
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            ("Ctl.java", &ctl("return svc.run();", "")),
            ("Svc.java", &svc("@Service", PRE_B)),
        ],
    ) else {
        return;
    };
    super::ui_flow_service::DROPPED.with(|d| d.borrow_mut().clear());
    assert!(
        (flow.diagnostics.iter()).any(|d| d.message.contains("missing from the call model")),
        "{:#?}",
        flow.diagnostics
    );
}

/// A call on a project bean that the compiler could not resolve (its argument
/// does not compile) must not vanish: its check is unknown.
#[test]
fn an_unresolved_call_on_a_project_bean_leaves_a_gap() {
    let Some(flow) = run(
        SPRING_POM,
        &[
            ("Conf.java", ENABLE),
            (
                "Ctl.java",
                &ctl("String s = \"x\"; s.trim(); return svc.run(Missing.X);", ""),
            ),
            (
                "Svc.java",
                &svc("@Service", &PRE_B.replace("run()", "run(Object o)")),
            ),
        ],
    ) else {
        return;
    };
    let gaps: Vec<_> = (flow.diagnostics.iter())
        .filter(|d| d.message.contains("not resolved"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(gaps.len(), 1, "{:#?}", flow.diagnostics);
    assert!(gaps[0].contains("svc.run"), "{gaps:?}");
}

/// One failed part of the call model is a gap while the others stay read.
#[test]
fn a_failed_part_of_the_call_model_is_a_gap() {
    let analyzer = |status: &str| archguard_core::CallAnalyzer {
        language: "java".into(),
        backend: "javac".into(),
        status: status.into(),
        files: 7,
        version: None,
        project: None,
        diagnostics: vec!["boom".into()],
    };
    let analysis = archguard_core::CallAnalysis {
        analyzers: vec![analyzer("ready"), analyzer("unavailable")],
        ..Default::default()
    };
    let gaps = super::ui_flow_service::failed_scopes(&analysis);
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert!(gaps[0].2.contains("7 Java file(s)") && gaps[0].2.contains("boom"));
}

fn pom(artifact: &str, deps: &[&str]) -> String {
    let deps: String = (deps.iter())
        .map(|d| format!("<dependency><artifactId>{d}</artifactId></dependency>"))
        .collect();
    format!(
        "<project><artifactId>{artifact}</artifactId><dependencies>{deps}</dependencies></project>"
    )
}

/// The interface with the check, the controller and the switch live in
/// three modules of one application (hawkbit): the check still applies.
#[test]
fn a_check_and_its_switch_in_other_modules_of_the_application_are_read() {
    let api = "package app;\nimport org.springframework.security.access.prepost.*;\npublic interface Api {\n  @PreAuthorize(\"hasRole('B')\") String run();\n}\n";
    let svc = "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass Svc implements Api {\n  public String run() { return \"\"; }\n}\n";
    let ctl = ctl("return svc.run();", "").replace("private Svc svc", "private Api svc");
    let m = "../../../../";
    let poms = [
        pom("api", &[]),
        pom("core", &["api", "base"]),
        pom("base", &[]),
        pom("web", &["api"]),
        pom("top", &["web", "core"]),
    ];
    let files = [
        (format!("{m}api/pom.xml"), poms[0].as_str()),
        (format!("{m}api/src/main/java/app/Api.java"), api),
        (format!("{m}core/pom.xml"), poms[1].as_str()),
        (format!("{m}base/pom.xml"), poms[2].as_str()),
        (format!("{m}base/src/main/java/app/Conf.java"), ENABLE),
        (format!("{m}core/src/main/java/app/Svc.java"), svc),
        (format!("{m}web/pom.xml"), poms[3].as_str()),
        (format!("{m}web/src/main/java/app/Ctl.java"), ctl.as_str()),
        (format!("{m}top/pom.xml"), poms[4].as_str()),
    ];
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(n, t)| (n.as_str(), *t)).collect();
    let Some(flow) = run(SPRING_POM, &borrowed) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{:#?}", flow.diagnostics);
    assert_eq!(l[0].0, ServiceHow::Enforced, "{l:?}");
    assert_eq!(flow.effective_access(endpoint(&flow)), role("ROLE_B"));
}

const API: &str = "package app;\nimport org.springframework.security.access.prepost.*;\npublic interface Api {\n  @PreAuthorize(\"hasRole('B')\") String run();\n}\n";
const IMPL: &str = "package app;\nimport org.springframework.stereotype.Service;\n@Service\nclass Impl implements Api {\n  public String run() { return \"\"; }\n}\n";

/// Modules `(name, pom deps, [(file, text)])` placed beside the project root.
type Module<'a> = (&'a str, &'a [&'a str], Vec<(&'a str, &'a str)>);

fn modules(mods: &[Module]) -> Option<UiFlow> {
    let mut files: Vec<(String, String)> = Vec::new();
    for (name, deps, list) in mods {
        files.push((format!("../../../../{name}/pom.xml"), pom(name, deps)));
        for (p, t) in list {
            let path = format!("../../../../{name}/src/main/java/app/{p}");
            files.push((path, t.to_string()));
        }
    }
    let borrowed: Vec<(&str, &str)> = (files.iter())
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();
    run(SPRING_POM, &borrowed)
}

fn gap_with(flow: &UiFlow, word: &str) -> bool {
    (flow.diagnostics.iter())
        .any(|d| d.severity == archguard_core::DiagnosticSeverity::Gap && d.message.contains(word))
}

/// Two modules declare `app.Svc`; the caller depends on the unchecked one and
/// must never be shown the checked twin's annotation.
#[test]
fn a_call_to_a_class_declared_in_two_modules_is_a_gap_not_a_check() {
    let twin = |check: &str| {
        format!(
            "package app;\nimport org.springframework.stereotype.Service;\nimport org.springframework.security.access.prepost.*;\n@Service\npublic class Svc {{\n  {check} public String run() {{ return \"\"; }}\n}}\n"
        )
    };
    let (a, b) = (twin("@PreAuthorize(\"hasRole('B')\")"), twin(""));
    let Some(flow) = modules(&[
        ("a", &[], vec![("Svc.java", &a), ("Conf.java", ENABLE)]),
        ("b", &[], vec![("Svc.java", &b), ("Conf.java", ENABLE)]),
        (
            "c",
            &["b"],
            vec![("Ctl.java", &ctl("return svc.run();", ""))],
        ),
    ]) else {
        return;
    };
    assert!(layers(&flow).is_empty(), "{:#?}", flow.service_layers);
    assert!(
        gap_with(&flow, "more than one module"),
        "{:#?}",
        flow.diagnostics
    );
}

/// No application holds both the endpoint's module and the service's module.
#[test]
fn a_service_outside_every_application_of_the_endpoint_is_a_hint_and_a_gap() {
    let ctl = ctl("return svc.run();", "").replace("private Svc svc", "private Api svc");
    let Some(flow) = modules(&[
        ("api", &[], vec![("Api.java", API)]),
        ("base", &[], vec![("Conf.java", ENABLE)]),
        ("core", &["api", "base"], vec![("Impl.java", IMPL)]),
        ("web", &["api"], vec![("Ctl.java", &ctl)]),
    ]) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{:#?}", flow.diagnostics);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert!(gap_with(&flow, "application"), "{:#?}", flow.diagnostics);
}

/// Two applications include endpoint and service but only one enables method security.
#[test]
fn applications_that_disagree_on_method_security_give_a_hint_and_a_gap() {
    let ctl = ctl("return svc.run();", "").replace("private Svc svc", "private Api svc");
    let Some(flow) = modules(&[
        ("api", &[], vec![("Api.java", API)]),
        ("base", &[], vec![("Conf.java", ENABLE)]),
        ("core", &["api"], vec![("Impl.java", IMPL)]),
        ("web", &["api", "core"], vec![("Ctl.java", &ctl)]),
        ("top1", &["web", "base"], vec![]),
        ("top2", &["web"], vec![]),
    ]) else {
        return;
    };
    let l = layers(&flow);
    assert_eq!(l.len(), 1, "{:#?}", flow.diagnostics);
    assert_eq!(l[0].0, ServiceHow::Hint, "{l:?}");
    assert!(gap_with(&flow, "differ"), "{:#?}", flow.diagnostics);
}

/// A walk that fails after the controllers were read (the endpoints stay) must
/// say why the service checks were not made, as a note: the flow is already
/// incomplete through the reader that built it, so a second gap adds nothing.
#[test]
fn unreadable_discovery_settings_are_noted_in_the_service_pass() {
    let dir = tempfile::tempdir().unwrap();
    let ctl = dir.path().join("svc-walk/src/main/java/app/Ctl.java");
    std::fs::create_dir_all(ctl.parent().unwrap()).unwrap();
    std::fs::write(dir.path().join("svc-walk/pom.xml"), SPRING_POM).unwrap();
    std::fs::write(&ctl, ctl_src()).unwrap();
    let root = dir.path().join("svc-walk");
    let mut flow = crate::ui_flow::analyze(&root).unwrap();
    assert!(flow.endpoints.iter().any(|e| e.handler.is_some()));
    std::fs::write(root.join("archguard.toml"), b"[project]\nexclude = 1\n").unwrap();
    let config = archguard_core::Config::default();
    let ctx = archguard_flow_common::extract_ctx::ExtractContext::new(&root, &config);
    let complete = flow.complete;
    crate::ui_flow_service::apply(&ctx, &mut flow);
    let note = flow
        .diagnostics
        .iter()
        .find(|d| d.message.contains("ejb-jar.xml"))
        .unwrap_or_else(|| panic!("no diagnostic names the walk: {:?}", flow.diagnostics));
    assert_eq!(note.severity, archguard_core::DiagnosticSeverity::Note);
    assert_eq!(flow.complete, complete, "a note must not flip `complete`");
    // This fixture has no SIGNS and, with the walk failed, no descriptor either, so
    // `apply` returns before reading anything. The note must not promise a reading
    // that does not happen on this path.
    assert!(
        note.message.contains("unknown"),
        "the note must name what stays unknown: {}",
        note.message
    );
}

fn ctl_src() -> String {
    ctl("return \"x\";", "")
}
