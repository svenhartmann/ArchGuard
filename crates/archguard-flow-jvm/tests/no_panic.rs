//! The JVM readers take foreign source: broken input must never panic.
#[path = "../../archguard-flow-common/tests/support/dirs.rs"]
mod dirs;
#[path = "../../archguard-flow-common/tests/support/variants.rs"]
mod variants;

use archguard_flow_jvm::{domain_java, ui_flow_micronaut, ui_flow_quarkus};

const POM: &str = "<project><dependencies><dependency><artifactId>quarkus-resteasy</artifactId></dependency></dependencies></project>";
const JAVA: &str = r#"
package a.b;
import jakarta.ws.rs.*;
import jakarta.persistence.*;
@Path("/orders")
@RolesAllowed({"admin"})
public class OrderResource {
    @Inject OrderService svc;
    @GET @Path("/{id}") public Order get(@PathParam("id") Long id) { return svc.find(id); }
    @POST public void add(Order o) { svc.save(o); }
}
@Entity public class Order { @Id Long id; @OneToMany List<Line> lines; String name; }
"#;
const KOTLIN: &str = r#"
package a
@Controller("/k")
class K(val svc: S) {
    @Get("/{id}") @Secured("ROLE_X") fun one(id: Long): String = svc.f(id)
    @Post fun add(o: Order) { svc.save(o) }
}
data class Order(val id: Long, val name: String)
"#;

#[test]
fn jvm_extractors_survive_broken_files() {
    let files = [
        ("pom.xml", POM),
        ("src/main/java/OrderResource.java", JAVA),
        ("src/main/kotlin/K.kt", KOTLIN),
        (
            "src/main/resources/application.properties",
            "quarkus.http.root-path=/x\nmicronaut.a=b\n",
        ),
        (
            "src/main/resources/application.yml",
            "micronaut:\n  application:\n    name: x\n",
        ),
    ];
    dirs::each_broken_tree(&files, |root| {
        ui_flow_quarkus::extract(root);
        ui_flow_micronaut::extract(root);
        domain_java::extract(root);
    });
}
