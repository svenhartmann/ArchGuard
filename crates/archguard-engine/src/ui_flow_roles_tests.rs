//! Role and permission catalog rules, on inline fixtures.
use crate::ui_flow_roles::extract;
use archguard_core::{Assignment, Condition, RoleKind, UiFlow, resolve_assignment};

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

/// `name kind file:line [includes]` per role def.
fn roles(flow: &UiFlow) -> Vec<String> {
    (flow.roles.iter())
        .map(|d| {
            let kind = match d.kind {
                RoleKind::Role => "role",
                RoleKind::Permission => "permission",
                RoleKind::Group => "group",
                RoleKind::Scope => "scope",
            };
            format!("{} {kind} {}:{} {:?}", d.name, d.file, d.line, d.includes)
        })
        .collect()
}

fn policy<'a>(flow: &'a UiFlow, name: &str) -> &'a Condition {
    &(flow.policies.iter().find(|p| p.name == name))
        .unwrap_or_else(|| panic!("{name} in {:#?}", flow.policies))
        .requires
}

fn notes(flow: &UiFlow) -> String {
    let notes: Vec<String> = (flow.diagnostics.iter())
        .map(|d| format!("{:?} {}:{} {}", d.severity, d.file, d.line, d.message))
        .collect();
    notes.join("\n")
}

fn holding(role: &str) -> Assignment {
    Assignment {
        authenticated: true,
        roles: [role.to_string()].into(),
        ..Assignment::default()
    }
}

const ROLES_CS: &str = "namespace Shop.Domain;\n\npublic abstract class Roles\n{\n    public const string Administrator = nameof(Administrator);\n}\n";
const POLICIES_CS: &str =
    "public static class Policies\n{\n    public const string CanPurge = \"CanPurge\";\n}\n";
const STARTUP_CS: &str = r#"public static class DependencyInjection
{
    public static void Add(IServiceCollection services, string name)
    {
        services.AddAuthorizationBuilder()
            .AddPolicy(Policies.CanPurge, policy => policy.RequireRole(Roles.Administrator));
        services.AddAuthorization(options =>
        {
            options.AddPolicy("Api", p =>
            {
                p.RequireAuthenticatedUser();
                p.RequireClaim("scope", "api", "web");
            });
            options.AddPolicy("Odd", p => p.RequireAssertion(ctx => true));
            options.AddPolicy(name, p => p.RequireRole("x"));
            // options.AddPolicy("Commented", p => p.RequireRole("y"));
            options.AddPolicy("Builder", new AuthorizationPolicyBuilder().RequireRole("A", "B").Build());
            options.FallbackPolicy = new AuthorizationPolicyBuilder().RequireAuthenticatedUser().Build();
        });
    }
}
"#;
const SEED_CS: &str = r#"public class Seeder
{
    public async Task Seed(string name)
    {
        var admin = new IdentityRole(Roles.Administrator);
        await _roleManager.CreateAsync(new IdentityRole("Auditor"));
        await _roleManager.CreateAsync(new IdentityRole(name));
    }
}
"#;
const ENUM_CS: &str = "public enum UserType\n{\n    Owner = 0,\n    Admin = 1,\n}\n\npublic class AdminController\n{\n    [Authorize(Roles = nameof(UserType.Admin))]\n    public IActionResult Get() => Ok();\n}\n";

#[test]
fn aspnet_policies_role_constants_seeding_and_enums() {
    let dir = project(&[
        ("src/Domain/Roles.cs", ROLES_CS),
        ("src/Domain/Policies.cs", POLICIES_CS),
        ("src/Infrastructure/DependencyInjection.cs", STARTUP_CS),
        ("src/Infrastructure/Seeder.cs", SEED_CS),
        ("src/Web/AdminController.cs", ENUM_CS),
        (
            "tests/Shop.Tests/TestRoles.cs",
            "public static class Roles { public const string Hidden = \"Hidden\"; }",
        ),
    ]);
    let flow = extract(dir.path());
    assert_eq!(
        roles(&flow),
        [
            "Admin role src/Web/AdminController.cs:4 []",
            "Administrator role src/Domain/Roles.cs:5 []",
            "Administrator role src/Infrastructure/Seeder.cs:5 []",
            "Auditor role src/Infrastructure/Seeder.cs:6 []",
        ],
        "{}",
        notes(&flow)
    );
    let role = |n: &str| Condition::Role(n.into());
    assert_eq!(policy(&flow, "CanPurge"), &role("Administrator"));
    assert_eq!(
        policy(&flow, "Api"),
        &Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("claim scope = api|web".into())
        ])
    );
    assert_eq!(
        policy(&flow, "Odd"),
        &Condition::Unknown("RequireAssertion".into())
    );
    assert_eq!(
        policy(&flow, "Builder"),
        &Condition::Any(vec![role("A"), role("B")])
    );
    assert_eq!(policy(&flow, "FallbackPolicy"), &Condition::Authenticated);
    assert_eq!(flow.policies.len(), 5, "{:#?}", flow.policies);
    let can_purge = flow.policies.iter().find(|p| p.name == "CanPurge").unwrap();
    assert_eq!(
        (can_purge.file.as_str(), can_purge.line),
        ("src/Infrastructure/DependencyInjection.cs", 6)
    );
    let notes = notes(&flow);
    for expected in [
        "Gap src/Infrastructure/DependencyInjection.cs:14 Policy requirement not readable: RequireAssertion",
        "Gap src/Infrastructure/DependencyInjection.cs:15 Policy name not readable: name",
        "Gap src/Infrastructure/Seeder.cs:7 Seeded role name not readable: name",
    ] {
        assert!(notes.contains(expected), "{expected} in\n{notes}");
    }
    assert!(
        !flow.complete,
        "unreadable declarations make the flow incomplete"
    );
}

const AUTHORITIES_JAVA: &str = "package x.security;\n\npublic final class AuthoritiesConstants {\n\n    public static final String ADMIN = \"ROLE_ADMIN\";\n\n    public static final String USER = \"ROLE_USER\";\n\n    private AuthoritiesConstants() {}\n}\n";
const PERMS_JAVA: &str = "public final class Perms {\n    public static final String AUDIT = \"ROLE_AUDITOR\";\n    public static final String UNUSED = \"ROLE_UNUSED\";\n}\n";
const SECURITY_JAVA: &str = r#"public class SecurityConfiguration {
    @Bean
    RoleHierarchy roleHierarchy() {
        return RoleHierarchyImpl.fromHierarchy("ROLE_ADMIN > ROLE_STAFF\n" + "ROLE_STAFF > ROLE_USER");
    }
    void other(RoleHierarchyImpl h) {
        h.setHierarchy(hierarchyText);
    }
    SecurityFilterChain chain(HttpSecurity http) {
        http.authorizeHttpRequests(a -> a.requestMatchers("/audit/**").hasAuthority(Perms.AUDIT));
    }
    @PreAuthorize("hasAuthority(\"" + AuthoritiesConstants.ADMIN + "\")")
    public void delete() {}
}
"#;

#[test]
fn spring_constants_hierarchy_and_kotlin_objects() {
    let dir = project(&[
        (
            "src/main/java/x/AuthoritiesConstants.java",
            AUTHORITIES_JAVA,
        ),
        ("src/main/java/x/Perms.java", PERMS_JAVA),
        ("src/main/java/x/SecurityConfiguration.java", SECURITY_JAVA),
        (
            "src/main/kotlin/x/Roles.kt",
            "object Roles {\n    const val EDITOR: String = \"ROLE_EDITOR\"\n}\n",
        ),
    ]);
    let flow = extract(dir.path());
    let sec = "src/main/java/x/SecurityConfiguration.java";
    assert_eq!(
        roles(&flow),
        [
            "ROLE_ADMIN role src/main/java/x/AuthoritiesConstants.java:5 []".into(),
            format!("ROLE_ADMIN role {sec}:4 [\"ROLE_STAFF\"]"),
            "ROLE_AUDITOR role src/main/java/x/Perms.java:2 []".into(),
            format!("ROLE_STAFF role {sec}:4 [\"ROLE_USER\"]"),
        ],
        "{}",
        notes(&flow)
    );
    assert!(
        notes(&flow).contains(&format!(
            "Gap {sec}:7 Role hierarchy not readable: hierarchyText"
        )),
        "{}",
        notes(&flow)
    );
    let resolved = resolve_assignment(&flow, &holding("ROLE_ADMIN"));
    assert!(resolved.roles.contains("ROLE_USER"), "{resolved:?}");
}

#[test]
fn frontend_enums_and_const_objects_used_in_code() {
    let dir = project(&[
        (
            "web/app/constants.ts",
            "// export enum Authority { GHOST = 'ROLE_GHOST' }\nexport enum Authority {\n  ADMIN = 'ROLE_ADMIN',\n  USER = 'ROLE_USER',\n}\n",
        ),
        (
            "web/app/app.routes.ts",
            "export const routes = [{ path: 'admin', data: { authorities: [Authority.ADMIN] } }];\nconst x = Permissions.Computed;\n",
        ),
        (
            "web/app/permissions.ts",
            "export const Permissions = {\n  ReadOrders: 'orders.read',\n  Computed: prefix + '.x',\n  Unused: 'never.used',\n} as const;\n",
        ),
        (
            "web/app/orders.html",
            "<button *hasPermission=\"Permissions.ReadOrders\">Read</button>\n",
        ),
    ]);
    let flow = extract(dir.path());
    assert_eq!(
        roles(&flow),
        [
            "ROLE_ADMIN role web/app/constants.ts:3 []",
            "orders.read permission web/app/permissions.ts:2 []",
        ],
        "{}",
        notes(&flow)
    );
    assert!(
        notes(&flow).contains(
            "web/app/permissions.ts:3 Permissions.Computed is not a string literal: prefix + '.x'"
        ),
        "{}",
        notes(&flow)
    );
}

fn cs_policies(body: &str) -> UiFlow {
    let text = format!("class S {{ void C(O o) {{\n{body}\n}} }}");
    extract(project(&[("S.cs", text.as_str())]).path())
}

#[test]
fn require_role_arguments_are_roles_not_comma_lists() {
    let flow = cs_policies(
        r#"o.AddPolicy("P", p => p.RequireRole("A,B"));
o.AddPolicy("Q", p => p.RequireRole("A", "B"));
o.AddPolicy("R", p => p.RequireRole("A").RequireRole("B"));"#,
    );
    let role = |n: &str| Condition::Role(n.into());
    assert_eq!(policy(&flow, "P"), &role("A,B"));
    assert_eq!(
        policy(&flow, "Q"),
        &Condition::Any(vec![role("A"), role("B")])
    );
    assert_eq!(
        policy(&flow, "R"),
        &Condition::All(vec![role("A"), role("B")])
    );
}

#[test]
fn statement_lambda_and_requirements_add() {
    let flow = cs_policies(
        r#"o.AddPolicy("P", p => { p.RequireRole("A"); p.RequireClaim("scope", "x", "y"); });
o.AddPolicy("M", p => p.Requirements.Add(new MinimumAgeRequirement(21)));"#,
    );
    assert_eq!(
        policy(&flow, "P"),
        &Condition::All(vec![
            Condition::Role("A".into()),
            Condition::Custom("claim scope = x|y".into())
        ])
    );
    assert_eq!(
        policy(&flow, "M"),
        &Condition::Custom("MinimumAgeRequirement(21)".into())
    );
}

#[test]
fn unreadable_policy_parts_are_gaps() {
    let flow =
        cs_policies(r#"o.AddPolicy("P", p => p.RequireAssertion(c => c.User.IsInRole("A")));"#);
    assert_eq!(
        policy(&flow, "P"),
        &Condition::Unknown("RequireAssertion".into())
    );
    assert!(
        notes(&flow).starts_with("Gap S.cs:2 Policy requirement not readable"),
        "{}",
        notes(&flow)
    );
    assert!(!flow.complete);
}

#[test]
fn no_catalog_from_class_names() {
    let dir = project(&[
        (
            "Roles.cs",
            "public static class Roles { public const string Unused = \"Unused\"; public const string Used = \"Used\"; }",
        ),
        (
            "PermissionIds.cs",
            "public static class PermissionIds { public const string X = \"a.b\"; }",
        ),
        (
            "C.cs",
            "class C { [Authorize(Roles = Roles.Used)] void A() {} }",
        ),
        (
            "AuthoritiesConstants.java",
            "class AuthoritiesConstants { static final String ADMIN = \"ROLE_ADMIN\"; }",
        ),
    ]);
    assert_eq!(roles(&extract(dir.path())), ["Used role Roles.cs:1 []"]);
}

#[test]
fn aspnet_convention_is_read_per_application() {
    let dir = project(&[
        ("Web/Web.csproj", "<Project/>"),
        (
            "Web/Startup.cs",
            "class S {\n void C(O o) { o.AddPolicy(\"RequireAdminRole\", p => p.RequireRole(\"Admin\")); }\n void T() { var t = new TokenValidationParameters { RoleClaimType = \"roles\" }; }\n}",
        ),
        ("Other/Other.csproj", "<Project/>"),
        (
            "Other/P.cs",
            "class P { void C(O o) { o.AddPolicy(\"Q\", p => p.RequireRole(\"x\")); } }",
        ),
    ]);
    let flow = extract(dir.path());
    let web = flow.convention(Some("Web"));
    assert_eq!(web.policy_case, archguard_core::NameCase::Insensitive);
    assert_eq!(web.role_case, archguard_core::NameCase::Exact);
    assert_eq!(web.role_claim.as_deref(), Some("roles"));
    let claim = web
        .origins
        .iter()
        .find(|o| o.field == "role_claim")
        .unwrap();
    assert_eq!((claim.file.as_str(), claim.line), ("Web/Startup.cs", 3));
    let other = (flow.role_conventions.iter())
        .find(|c| c.application.as_deref() == Some("Other"))
        .unwrap();
    assert_eq!(other.role_claim, None);
    let origin = (other.origins.iter())
        .find(|o| o.field == "role_claim")
        .unwrap();
    assert_eq!(origin.source, "not read (exact match)");
    assert!(
        flow.role_conventions
            .iter()
            .all(|c| c.application.is_some())
    );
}

#[test]
fn convention_read_outside_every_project_is_a_gap_not_a_whole_flow_entry() {
    let dir = project(&[(
        "S.cs",
        "class S { void T() { var t = new TokenValidationParameters { RoleClaimType = \"roles\" }; } }",
    )]);
    let flow = extract(dir.path());
    assert!(
        flow.role_conventions.is_empty(),
        "{:?}",
        flow.role_conventions
    );
    assert!(
        notes(&flow).contains("Gap S.cs:1 Role claim type"),
        "{}",
        notes(&flow)
    );
    assert!(!flow.complete);
}

#[test]
fn spring_role_hierarchy_builder_and_authority_target() {
    let dir = project(&[
        ("api/pom.xml", "<project/>"),
        (
            "api/src/Sec.java",
            "class Sec {\n  Object a() { return RoleHierarchyImpl.withDefaultRolePrefix().role(\"ADMIN\").implies(\"STAFF\").role(\"STAFF\").implies(\"USER\").build(); }\n  Object b() { return RoleHierarchyImpl.fromHierarchy(\"ROLE_ADMIN > permission:read\"); }\n}",
        ),
    ]);
    let flow = extract(dir.path());
    let includes = |name: &str| -> Vec<Vec<String>> {
        (flow.roles.iter().filter(|r| r.name == name))
            .map(|r| r.includes.clone())
            .collect()
    };
    assert_eq!(
        includes("ROLE_ADMIN"),
        [
            vec!["ROLE_STAFF".to_string()],
            vec!["permission:read".to_string()]
        ]
    );
    assert_eq!(includes("ROLE_STAFF"), [vec!["ROLE_USER".to_string()]]);
    assert!(flow.roles.iter().all(|r| r.name != "permission:read"));
    let c = flow.convention(Some("api"));
    assert_eq!(c.role_prefix.as_deref(), Some("ROLE_"));
    assert_eq!(c.origins[0].line, 2);
}

// F44: a right-hand side without the role prefix is a permission, never a role def.
#[test]
fn role_hierarchy_text_may_include_a_permission() {
    let java = "class H { RoleHierarchy a() { return RoleHierarchyImpl.fromHierarchy(\"ROLE_ADMIN > permission:read\"); } }\n";
    let dir = project(&[("src/main/java/H.java", java)]);
    let flow = extract(dir.path());
    assert_eq!(
        roles(&flow),
        ["ROLE_ADMIN role src/main/java/H.java:1 [\"permission:read\"]"],
        "{}",
        notes(&flow)
    );
}

// W5 guard: a literal withRolePrefix names the roles and is the read prefix.
#[test]
fn with_role_prefix_literal_names_roles_and_sets_the_prefix() {
    let java = "class Sec { Object a() { return RoleHierarchyImpl.withRolePrefix(\"X_\").role(\"ADMIN\").implies(\"STAFF\").build(); } }";
    let dir = project(&[("api/pom.xml", "<project/>"), ("api/src/Sec.java", java)]);
    let flow = extract(dir.path());
    assert_eq!(
        roles(&flow),
        ["X_ADMIN role api/src/Sec.java:1 [\"X_STAFF\"]"],
        "{}",
        notes(&flow)
    );
    assert_eq!(
        flow.convention(Some("api")).role_prefix.as_deref(),
        Some("X_")
    );
}

// W5 guard: an unreadable builder argument is a gap, never a guessed prefix.
#[test]
fn unreadable_with_role_prefix_argument_is_a_gap() {
    let java = "class Sec { Object a() { return RoleHierarchyImpl.withRolePrefix(prefix()).role(\"ADMIN\").implies(\"STAFF\").build(); } }";
    let dir = project(&[("api/pom.xml", "<project/>"), ("api/src/Sec.java", java)]);
    let flow = extract(dir.path());
    assert!(roles(&flow).is_empty());
    assert_eq!(flow.convention(Some("api")).role_prefix, None);
    assert!(!flow.complete);
    assert!(notes(&flow).contains("Role hierarchy prefix not readable"));
}

// W5 item 1: two different prefixes in one app are never resolved by picking one.
#[test]
fn two_different_role_prefixes_in_one_app_stay_unread_with_a_gap() {
    let java = "class Sec {\n  Object a() { return RoleHierarchyImpl.withRolePrefix(\"X_\").role(\"A\").implies(\"B\").build(); }\n  Object b() { return RoleHierarchyImpl.withRolePrefix(\"Y_\").role(\"A\").implies(\"B\").build(); }\n}";
    let dir = project(&[("api/pom.xml", "<project/>"), ("api/src/Sec.java", java)]);
    let flow = archguard_core::merge_ui_flows(extract(dir.path()), UiFlow::default());
    assert_eq!(flow.convention(Some("api")).role_prefix, None);
    assert!(!flow.complete);
    assert!(notes(&flow).contains("role_prefix"), "{}", notes(&flow));
}
