//! The backend access readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_backend::{ui_flow_backend, ui_flow_webflux};

const SPRING: &str = r#"
@RestController
@RequestMapping("/api")
public class C {
    @PreAuthorize("hasRole('ADMIN')") @GetMapping("/a/{id}") public String a(@PathVariable Long id) { return svc.a(id); }
    @PostMapping(path = {"/b", "/c"}) public void b() {}
}
@Configuration class Sec { SecurityFilterChain f(HttpSecurity h) { return h.authorizeHttpRequests(a -> a.requestMatchers("/x/**").hasRole("U").anyRequest().authenticated()).build(); } }
@Bean RouterFunction<ServerResponse> r() { return route().GET("/w", h::get).build(); }
"#;
const ASPNET: &str = r#"
[ApiController][Route("api/[controller]")][Authorize(Roles = "admin")]
public class OrdersController : ControllerBase {
    [HttpGet("{id}")] public IActionResult Get(int id) => Ok();
    [AllowAnonymous][HttpPost] public IActionResult Add() => Ok();
}
app.MapGet("/min", () => "x").RequireAuthorization("p");
"#;

#[test]
fn backend_extractors_survive_broken_files() {
    let files = [
        (
            "pom.xml",
            "<project><artifactId>spring-boot-starter-web</artifactId></project>",
        ),
        ("src/main/java/C.java", SPRING),
        ("src/main/kotlin/C.kt", SPRING),
        (
            "App.csproj",
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>",
        ),
        ("Controllers/OrdersController.cs", ASPNET),
        ("Program.cs", ASPNET),
    ];
    dirs::each_broken_tree(&files, |root| {
        ui_flow_backend::extract(root);
        ui_flow_webflux::extract(root);
    });
}
