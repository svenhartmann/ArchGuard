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
        "Note src/Infrastructure/DependencyInjection.cs:14 Policy requirement not readable: RequireAssertion",
        "Note src/Infrastructure/DependencyInjection.cs:15 Policy name not readable: name",
        "Note src/Infrastructure/Seeder.cs:7 Seeded role name not readable: name",
    ] {
        assert!(notes.contains(expected), "{expected} in\n{notes}");
    }
    assert!(flow.complete, "notes leave the flow complete");
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
            "ROLE_EDITOR role src/main/kotlin/x/Roles.kt:2 []".into(),
            format!("ROLE_STAFF role {sec}:4 [\"ROLE_USER\"]"),
            "ROLE_USER role src/main/java/x/AuthoritiesConstants.java:7 []".into(),
        ],
        "{}",
        notes(&flow)
    );
    assert!(
        notes(&flow).contains(&format!(
            "{sec}:7 Role hierarchy not readable: hierarchyText"
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

#[test]
fn permission_ids_are_permission_defs_and_wildcards_cover() {
    let dir = project(&[(
        "backend/src/Shared/PermissionIds.cs",
        "public static class PermissionIds\n{\n    public const string All = \"squidex.*\";\n    public const string AppContents = \"squidex.apps.{app}.contents.*\";\n    public const string AppContentsRead = \"squidex.apps.{app}.contents.read\";\n}\n",
    )]);
    let flow = extract(dir.path());
    assert_eq!(
        roles(&flow),
        [
            "squidex.* permission backend/src/Shared/PermissionIds.cs:3 []",
            "squidex.apps.{app}.contents.* permission backend/src/Shared/PermissionIds.cs:4 []",
            "squidex.apps.{app}.contents.read permission backend/src/Shared/PermissionIds.cs:5 []",
        ]
    );
    let resolved = resolve_assignment(&flow, &holding("squidex.*"));
    assert_eq!(resolved.roles.len(), 3, "{resolved:?}");
}
