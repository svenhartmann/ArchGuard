//! `archguard ui-flow` and `archguard domain` (ADR 0021, 0022).
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const FILES: &[(&str, &str)] = &[
    ("Web/Web.csproj", "<Project />"),
    (
        "Web/Pages/Home.razor",
        "@page \"/\"\n<a href=\"orders\">Orders</a>\n<a href=\"admin\">Admin</a>\n<a href=\"@ItemUrl()\">Item</a>\n",
    ),
    ("Web/Pages/Item.razor", "@page \"/item/{id:int}\"\n"),
    (
        "Web/Pages/Orders.razor",
        "@page \"/orders\"\n@attribute [Authorize]\n@code {\n    async Task Load() => await Http.GetFromJsonAsync<int[]>(\"api/orders\");\n}\n",
    ),
    (
        "Web/Pages/Admin.razor",
        "@page \"/admin\"\n@attribute [Authorize(Roles = \"Admin\")]\n",
    ),
    ("Api/Api.csproj", "<Project />"),
    (
        "Api/OrdersController.cs",
        "[ApiController]\n[Route(\"api/orders\")]\n[Authorize(Roles = \"Clerk\")]\npublic class OrdersController : ControllerBase\n{\n    [HttpGet]\n    public IActionResult List() => Ok();\n}\n",
    ),
    (
        "Api/Domain/Order.cs",
        r#"public interface IAggregateRoot { }
public enum OrderStatus { Submitted, Paid, Cancelled }
public class Order : IAggregateRoot
{
    public OrderStatus Status { get; private set; }
    public List<OrderItem> Items { get; } = new();
    public Order() { Status = OrderStatus.Submitted; }
    public void SetPaid()
    {
        if (Status == OrderStatus.Submitted)
        {
            Status = OrderStatus.Paid;
            AddDomainEvent(new OrderPaidEvent(Id));
        }
    }
    public void Cancel()
    {
        if (Status == OrderStatus.Paid) { throw new InvalidOperationException(); }
        Status = OrderStatus.Cancelled;
    }
}
public class OrderItem : Entity { public int Units { get; set; } }
public class PayOrderCommandHandler
{
    public void Handle(Order order) { order.SetPaid(); }
}
public class OrderPaidHandler : INotificationHandler<OrderPaidEvent> { }
"#,
    ),
    (
        "Api/Customers/Customer.cs",
        "public class Customer : IAggregateRoot { public string Name { get; set; } }\n",
    ),
];

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in FILES {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

fn run(dir: &Path, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(args)
        .arg(dir)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{stderr}");
    String::from_utf8(output.stdout).unwrap()
}

fn json(dir: &Path, args: &[&str]) -> Value {
    let mut all = args.to_vec();
    all.extend(["--format", "json"]);
    serde_json::from_str(&run(dir, &all)).unwrap()
}

fn line_with<'a>(text: &'a str, needle: &str) -> &'a str {
    (text.lines().find(|l| l.contains(needle))).unwrap_or_else(|| panic!("{needle} in:\n{text}"))
}

#[test]
fn ui_flow_text_lists_screens_by_application_and_findings() {
    let dir = project();
    let text = run(dir.path(), &["ui-flow"]);
    assert!(
        text.starts_with(
            "Screens 4 · navigations 2 of 3 resolved · actions 1, 0 unresolved · endpoints: 1 protected, 0 open, 0 undeclared · complete: no\n"
        ),
        "{text}"
    );
    assert!(text.contains("application Web\n"), "{text}");
    let admin = line_with(&text, "/admin");
    assert!(
        admin.contains("access: role Admin") && admin.contains("Web/Pages/Admin.razor:"),
        "{admin}"
    );
    assert!(line_with(&text, "/orders").contains("access: signed in"));
    assert!(line_with(&text, "Home").contains("access: none"));
    let finding = line_with(&text, "shown_but_refused");
    assert!(finding.contains("[warning]") && finding.contains("Web/Pages/Orders.razor:4"));
    assert!(!text.contains("Simulated"), "{text}");
}

#[test]
fn ui_flow_json_has_summary_findings_and_flow() {
    let dir = project();
    let value = json(dir.path(), &["ui-flow"]);
    assert_eq!(value["summary"]["screens"], 4);
    assert_eq!(value["summary"]["endpoints_by_access"]["protected"], 1);
    assert_eq!(value["flow"]["definition"], "ui-flow/v1");
    assert_eq!(value["flow"]["screens"].as_array().unwrap().len(), 4);
    assert!(
        (value["findings"].as_array().unwrap())
            .iter()
            .any(|f| f["kind"] == "shown_but_refused")
    );
    assert!(value.get("simulation").is_none(), "{value:#}");
}

fn reach_of<'a>(simulation: &'a Value, route: &str) -> &'a Value {
    let screens = simulation["screens"].as_array().unwrap();
    let screen = screens.iter().find(|s| s["route"] == route).unwrap();
    &screen["reach"]
}

#[test]
fn ui_flow_simulates_an_anonymous_user() {
    let dir = project();
    let text = run(dir.path(), &["ui-flow", "--anonymous"]);
    assert!(text.contains("Simulated: anonymous"), "{text}");
    assert!(line_with(&text, "Home").ends_with("· reached surely"));
    assert!(line_with(&text, "/orders").ends_with("· denied (needs: signed in)"));
    // Open, but only a link the rules could not resolve leads there.
    assert!(
        line_with(&text, "/item/").ends_with("· no known path (1 link in this app unresolved)"),
        "{text}"
    );
    assert!(line_with(&text, "GET /api/orders").starts_with("  denied "));
    let value = json(dir.path(), &["ui-flow", "--anonymous"]);
    let simulation = &value["simulation"];
    assert_eq!(simulation["anonymous"], true);
    assert_eq!(reach_of(simulation, "/"), "sure");
    assert_eq!(reach_of(simulation, "/orders"), "denied");
    let item = (simulation["screens"].as_array().unwrap().iter())
        .find(|s| s["route"] == "/item/{id:int}")
        .unwrap();
    assert_eq!(item["reach"], "no_known_path");
    assert_eq!(item["unresolved_transitions_in_application"], 1);
    assert_eq!(simulation["endpoints"][0]["outcome"], "denied");
}

#[test]
fn ui_flow_simulates_roles() {
    let dir = project();
    let text = run(dir.path(), &["ui-flow", "--role", "Admin"]);
    assert!(
        text.contains("Simulated: signed in with roles Admin"),
        "{text}"
    );
    assert!(line_with(&text, "/admin").ends_with("· reached surely"));
    assert!(line_with(&text, "GET /api/orders").starts_with("  denied "));
    let value = json(
        dir.path(),
        &["ui-flow", "--role", "Admin", "--role", "Clerk"],
    );
    let simulation = &value["simulation"];
    assert_eq!(simulation["roles"], serde_json::json!(["Admin", "Clerk"]));
    assert_eq!(reach_of(simulation, "/orders"), "sure");
    assert_eq!(simulation["endpoints"][0]["outcome"], "allowed");
}

#[test]
fn domain_text_shows_entities_state_machines_use_cases_and_events() {
    let dir = project();
    let text = run(dir.path(), &["domain"]);
    assert!(
        text.starts_with(
            "Entities 3 · state machines 1 · transitions 2 · use cases 1 · events 1 · complete: yes\n"
        ),
        "{text}"
    );
    assert!(line_with(&text, "aggregate_root Order ").contains("Api/Domain/Order.cs:3"));
    assert!(text.contains("  has_many OrderItem via Items\n"), "{text}");
    assert!(
        text.contains("State machine Order.Status (OrderStatus)\n"),
        "{text}"
    );
    assert!(text.contains("  initial: Submitted\n"), "{text}");
    let row = line_with(&text, "Submitted → Paid");
    assert!(
        row.contains("Order.SetPaid") && row.contains("OrderPaidEvent"),
        "{row}"
    );
    assert!(
        text.contains("  from → to · method · guard (allowed when) · events\n"),
        "{text}"
    );
    // The code throws when Paid; the guard reads as when the step is allowed.
    let cancel = line_with(&text, "Order.Cancel");
    assert!(
        cancel.starts_with(
            "  Submitted, Cancelled → Cancelled · Order.Cancel · !(Status == OrderStatus.Paid) · - · "
        ),
        "{cancel}"
    );
    let use_case = line_with(&text, "PayOrderCommandHandler");
    assert!(use_case.contains("calls Order.SetPaid"), "{use_case}");
    assert!(
        text.contains("OrderPaidEvent: raised by Order.SetPaid → handled by OrderPaidHandler"),
        "{text}"
    );
    assert!(text.contains("Customer"), "{text}");
}

#[test]
fn domain_json_and_entity_filter() {
    let dir = project();
    let value = json(dir.path(), &["domain"]);
    assert_eq!(value["summary"]["entities"], 3);
    assert_eq!(value["model"]["definition"], "domain-model/v1");
    let one = json(dir.path(), &["domain", "--entity", "Order"]);
    let ids: Vec<&str> = (one["model"]["entities"].as_array().unwrap())
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["Order"]);
    assert_eq!(one["model"]["use_cases"].as_array().unwrap().len(), 1);
    assert_eq!(one["model"]["events"].as_array().unwrap().len(), 1);
    let text = run(dir.path(), &["domain", "--entity", "Order"]);
    assert!(!text.contains("Customer"), "{text}");
    assert!(text.contains("has_many OrderItem"), "{text}");
    let none = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["domain", "--entity", "Nope"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(none.status.code(), Some(2));
}
