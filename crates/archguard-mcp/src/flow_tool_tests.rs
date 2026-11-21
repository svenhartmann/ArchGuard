//! `ui_flow` and `domain_model` against a tiny Blazor + ASP.NET + C# domain project.
use crate::domain_model_tools::DomainModelParams;
use crate::server::ArchguardTools;
use crate::ui_flow_tools::UiFlowParams;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};

const FILES: [(&str, &str); 14] = [
    ("Web/Web.csproj", "<Project />"),
    (
        "Web/Pages/Home.razor",
        "@page \"/\"\n<a href=\"orders\">Orders</a>\n<a href=\"@ItemUrl()\">Item</a>\n",
    ),
    ("Web/Pages/Item.razor", "@page \"/item/{id:int}\"\n"),
    (
        "Web/Pages/Orders.razor",
        "@page \"/orders\"\n@attribute [Authorize(Roles = \"Clerk\")]\n@code {\n    protected override async Task OnInitializedAsync()\n    {\n        await Http.GetFromJsonAsync<int[]>(\"api/orders\");\n    }\n}\n",
    ),
    ("Api/Api.csproj", "<Project />"),
    (
        "Api/OrdersController.cs",
        "[ApiController]\n[Route(\"api/orders\")]\n[Authorize(Roles = \"Clerk\")]\npublic class OrdersController : ControllerBase\n{\n    [HttpGet]\n    public IActionResult List() => Ok();\n}\n",
    ),
    (
        "Api/HealthController.cs",
        "[ApiController]\n[Route(\"api/health\")]\npublic class HealthController : ControllerBase\n{\n    [HttpGet]\n    public IActionResult Get() => Ok();\n}\n",
    ),
    (
        "Domain/Seed.cs",
        "namespace Shop.Seed;\npublic abstract class Entity { public void AddDomainEvent(object e) { } }\npublic interface IAggregateRoot { }\n",
    ),
    (
        "Domain/Order.cs",
        "namespace Shop.Domain;\npublic class Order : Entity, IAggregateRoot\n{\n    public OrderState State { get; private set; }\n    public Customer Buyer { get; private set; }\n    public Order() { State = OrderState.New; }\n    public void Submit()\n    {\n        if (State == OrderState.New)\n        {\n            AddDomainEvent(new OrderSubmitted(this));\n            State = OrderState.Submitted;\n        }\n    }\n    public void Cancel()\n    {\n        if (State == OrderState.Cancelled) { throw new InvalidOperationException(); }\n        State = OrderState.Cancelled;\n    }\n}\n",
    ),
    (
        "Domain/OrderState.cs",
        "namespace Shop.Domain;\npublic enum OrderState { New, Submitted, Cancelled }\n",
    ),
    (
        "Domain/Customer.cs",
        "namespace Shop.Domain;\npublic class Customer : Entity, IAggregateRoot\n{\n    public void Rename(string name) { }\n}\n",
    ),
    (
        "App/SubmitOrderCommandHandler.cs",
        "namespace Shop.App;\npublic class SubmitOrderCommandHandler : IRequestHandler<SubmitOrderCommand, bool>\n{\n    public async Task<bool> Handle(SubmitOrderCommand c, CancellationToken t)\n    {\n        var order = await _repo.GetAsync(c.Id);\n        order.Submit();\n        return true;\n    }\n}\n",
    ),
    (
        "Api/OrdersApi.cs",
        "public static class OrdersApi\n{\n    public static void Map(this IEndpointRouteBuilder app)\n    {\n        app.MapPost(\"api/submit\", SubmitAsync);\n    }\n    public static Task<bool> SubmitAsync(SubmitOrderCommand command, IMediator mediator) => mediator.Send(command);\n}\n",
    ),
    (
        "App/OrderSubmittedHandler.cs",
        "namespace Shop.App;\npublic class OrderSubmittedHandler : INotificationHandler<OrderSubmitted>\n{\n}\n",
    ),
];

fn project() -> (tempfile::TempDir, ArchguardTools) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in FILES {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let tools = ArchguardTools::new(dir.path().into());
    (dir, tools)
}

async fn ui_flow(tools: &ArchguardTools, args: Value) -> Result<Value, rmcp::ErrorData> {
    let params: UiFlowParams = serde_json::from_value(args).unwrap();
    let answer = tools.ui_flow(Parameters(params)).await?;
    assert!(answer.len() <= crate::tools::MAX_RESPONSE_BYTES);
    Ok(serde_json::from_str(&answer).unwrap())
}

async fn domain(tools: &ArchguardTools, args: Value) -> Result<Value, rmcp::ErrorData> {
    let params: DomainModelParams = serde_json::from_value(args).unwrap();
    let answer = tools.domain_model(Parameters(params)).await?;
    assert!(answer.len() <= crate::tools::MAX_RESPONSE_BYTES);
    Ok(serde_json::from_str(&answer).unwrap())
}

/// Every record over all pages, each continuation bound to the first revision.
async fn all_rows(tools: &ArchguardTools, tool: &str, mut args: Value) -> (Value, Vec<Value>) {
    args["limit"] = json!(2);
    let mut rows = Vec::new();
    loop {
        let page = match tool {
            "ui_flow" => ui_flow(tools, args.clone()).await,
            _ => domain(tools, args.clone()).await,
        }
        .unwrap();
        rows.extend(page["items"].as_array().unwrap().iter().cloned());
        args["expected_revision"] = page["metadata"]["revision"].clone();
        match page["next_offset"].as_u64() {
            Some(next) => args["offset"] = json!(next),
            None => return (page, rows),
        }
    }
}

fn of_kind<'a>(rows: &'a [Value], kind: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["kind"] == kind).collect()
}

fn screen<'a>(rows: &'a [Value], route: &str) -> &'a Value {
    (rows.iter())
        .find(|r| r["kind"] == "screen" && r["route"] == route)
        .unwrap_or_else(|| panic!("no screen {route}: {rows:#?}"))
}

fn endpoint<'a>(rows: &'a [Value], part: &str) -> &'a Value {
    (rows.iter())
        .find(|r| r["kind"] == "endpoint" && r["endpoint"]["path"].as_str().unwrap().contains(part))
        .unwrap_or_else(|| panic!("no endpoint {part}: {rows:#?}"))
}

#[test]
fn both_tools_are_registered_read_only_with_their_parameters() {
    let tools = ArchguardTools::full_router().list_all();
    for (name, params) in [
        (
            "ui_flow",
            &[
                "path",
                "roles",
                "anonymous",
                "application",
                "offset",
                "limit",
                "expected_revision",
            ][..],
        ),
        (
            "domain_model",
            &[
                "path",
                "entity",
                "chain",
                "state",
                "roles",
                "anonymous",
                "offset",
                "limit",
                "expected_revision",
            ][..],
        ),
    ] {
        let tool = tools.iter().find(|t| t.name == name).expect(name);
        let description = tool.description.as_deref().unwrap();
        assert!(description.contains("Read-only"), "{name}: {description}");
        assert!(
            description.contains("Static evidence"),
            "{name}: {description}"
        );
        let schema = serde_json::to_value(&tool.input_schema).unwrap();
        for p in params {
            assert!(schema["properties"].get(p).is_some(), "{name} lacks {p}");
        }
    }
    let ui = tools.iter().find(|t| t.name == "ui_flow").unwrap();
    assert!(
        ui.description
            .as_deref()
            .unwrap()
            .contains("undeclared never means allowed")
    );
}

#[tokio::test]
async fn ui_flow_lists_screens_endpoints_and_summary_without_simulation() {
    let (_dir, tools) = project();
    let (last, rows) = all_rows(&tools, "ui_flow", json!({})).await;
    assert_eq!(last["metadata"]["summary"]["screens"], 3);
    assert!(last["metadata"]["simulation"].is_null());
    let home = screen(&rows, "/");
    assert_eq!(home["file"], "Web/Pages/Home.razor");
    assert!(home.get("reach").is_none());
    assert_eq!(of_kind(&rows, "transition").len(), 2);
    assert_eq!(of_kind(&rows, "action").len(), 1);
    assert_eq!(
        endpoint(&rows, "health")["endpoint"]["access"],
        "undeclared"
    );
    assert!(endpoint(&rows, "orders").get("simulated").is_none());
}

#[tokio::test]
async fn ui_flow_simulates_anonymous_and_roles() {
    let (_dir, tools) = project();
    let (last, anon) = all_rows(&tools, "ui_flow", json!({"anonymous": true})).await;
    assert_eq!(last["metadata"]["simulation"]["anonymous"], true);
    assert_eq!(screen(&anon, "/")["reach"], "undeclared");
    assert_eq!(screen(&anon, "/orders")["reach"], "denied");
    let item = screen(&anon, "/item/{id:int}");
    assert_eq!(item["reach"], "no_known_path");
    assert_eq!(item["unresolved_transitions_in_application"], 1);
    assert_eq!(endpoint(&anon, "orders")["simulated"], "denied");
    assert_eq!(endpoint(&anon, "health")["simulated"], "unknown");

    let (_, clerk) = all_rows(&tools, "ui_flow", json!({"roles": ["Clerk"]})).await;
    assert_eq!(screen(&clerk, "/orders")["reach"], "undeclared");
    assert_eq!(endpoint(&clerk, "orders")["simulated"], "allowed");
    assert_eq!(endpoint(&clerk, "health")["simulated"], "unknown");

    let (_, other) = all_rows(&tools, "ui_flow", json!({"roles": []})).await;
    assert_eq!(screen(&other, "/orders")["reach"], "denied");
    assert_eq!(endpoint(&other, "orders")["simulated"], "denied");
}

#[tokio::test]
async fn ui_flow_rejects_contradictions_and_unknown_application() {
    let (_dir, tools) = project();
    for args in [
        json!({"anonymous": true, "roles": ["Clerk"]}),
        json!({"application": "Nope"}),
        json!({"roles": [""]}),
    ] {
        assert!(ui_flow(&tools, args.clone()).await.is_err(), "{args}");
    }
    let (_, web) = all_rows(&tools, "ui_flow", json!({"application": "Web"})).await;
    assert_eq!(of_kind(&web, "screen").len(), 3);
}

#[tokio::test]
async fn pages_are_bounded_and_bound_to_the_revision() {
    let (_dir, tools) = project();
    let first = ui_flow(&tools, json!({"limit": 1})).await.unwrap();
    assert_eq!(first["returned"], 1);
    assert_eq!(first["next_offset"], 1);
    let stale = json!({"offset": 1, "expected_revision": "0000000000000000"});
    assert!(ui_flow(&tools, stale.clone()).await.is_err());
    assert!(domain(&tools, stale).await.is_err());
    let (_, rows) = all_rows(&tools, "ui_flow", json!({})).await;
    assert_eq!(rows.len() as u64, first["total"].as_u64().unwrap());
}

#[tokio::test]
async fn domain_model_lists_the_whole_model() {
    let (_dir, tools) = project();
    let (last, rows) = all_rows(&tools, "domain_model", json!({})).await;
    assert_eq!(last["metadata"]["summary"]["entities"], 2);
    assert_eq!(last["metadata"]["summary"]["state_machines"], 1);
    for kind in [
        "entity",
        "relation",
        "state_machine",
        "transition",
        "use_case",
        "event",
    ] {
        assert!(!of_kind(&rows, kind).is_empty(), "no {kind}: {rows:#?}");
    }
    assert!(of_kind(&rows, "transition")[0].get("event_chain").is_none());
    let cancel = (of_kind(&rows, "transition").into_iter())
        .find(|t| t["transition"]["method"] == "Order.Cancel")
        .unwrap_or_else(|| panic!("{rows:#?}"));
    // The code throws when Cancelled; the guard reads as when the step is allowed.
    assert_eq!(
        cancel["transition"]["guard"],
        "!(State == OrderState.Cancelled)"
    );
    let limits = last["metadata"]["limitations"].as_str().unwrap();
    assert!(limits.contains("allowed when"), "{limits}");
}

#[tokio::test]
async fn domain_model_entity_filter_adds_event_chain_and_use_cases() {
    let (_dir, tools) = project();
    let (_, rows) = all_rows(&tools, "domain_model", json!({"entity": "Order"})).await;
    let entities = of_kind(&rows, "entity");
    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0]["entity"]["id"], "Order");
    let submit = (of_kind(&rows, "transition").into_iter())
        .find(|t| t["transition"]["method"] == "Order.Submit")
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert_eq!(submit["event_chain"][0]["event"], "OrderSubmitted");
    assert_eq!(
        submit["event_chain"][0]["handlers"],
        json!(["OrderSubmittedHandler"])
    );
    assert_eq!(submit["use_cases"], json!(["SubmitOrderCommandHandler"]));
    assert!(
        !rows
            .to_vec()
            .iter()
            .any(|r| r["kind"] == "entity" && r["entity"]["id"] == "Customer")
    );
    assert!(domain(&tools, json!({"entity": "Nope"})).await.is_err());
}

#[tokio::test]
async fn domain_model_chain_and_triggers_of_a_state() {
    let (_dir, tools) = project();
    let (last, rows) = all_rows(&tools, "domain_model", json!({"chain": true})).await;
    assert!(rows.iter().all(|r| r["kind"] == "chain"), "{rows:#?}");
    assert_eq!(rows.len() as u64, last["total"].as_u64().unwrap());
    let submit = (rows.iter())
        .find(|r| r["endpoint"]["path"] == "/api/submit")
        .unwrap_or_else(|| panic!("{rows:#?}"));
    assert_eq!(submit["endpoint"]["handler"], "OrdersApi.SubmitAsync");
    assert_eq!(submit["use_cases"][0]["id"], "SubmitOrderCommandHandler");
    assert_eq!(submit["transitions"][0]["transition"]["to"], "Submitted");
    assert_eq!(
        submit["events"][0],
        json!({"event": "OrderSubmitted", "handlers": ["OrderSubmittedHandler"]})
    );
    let (_, submitted) = all_rows(
        &tools,
        "domain_model",
        json!({"entity": "Order", "state": "Submitted"}),
    )
    .await;
    assert_eq!(submitted.len(), 1, "{submitted:#?}");
    assert_eq!(submitted[0]["endpoint"]["path"], "/api/submit");
    let (_, cancelled) = all_rows(
        &tools,
        "domain_model",
        json!({"entity": "Order", "state": "Cancelled"}),
    )
    .await;
    assert!(cancelled.is_empty(), "{cancelled:#?}");
    assert!(domain(&tools, json!({"state": "Submitted"})).await.is_err());
}

#[tokio::test]
async fn domain_model_triggers_with_message_handlers_and_rights() {
    let (dir, tools) = project();
    std::fs::write(
        dir.path().join("App/ImportHandler.cs"),
        "public class ImportHandler(IMediator mediator) : IIntegrationEventHandler<OrderImported>\n{\n    public Task Handle(OrderImported e) => mediator.Send(new SubmitOrderCommand(e.Id));\n}\n",
    )
    .unwrap();
    let args = json!({"entity": "Order", "state": "Submitted", "roles": ["Clerk"]});
    let (last, rows) = all_rows(&tools, "domain_model", args).await;
    assert_eq!(rows.len(), 2, "{rows:#?}");
    assert_eq!(rows[0]["endpoint"]["path"], "/api/submit");
    assert_eq!(
        rows[0]["right"],
        json!({"verdict": "unknown", "reasons": ["endpoint POST /api/submit no access declared: unknown"]})
    );
    assert_eq!(rows[1]["message_handler"]["message"], "OrderImported");
    assert_eq!(rows[1]["right"]["verdict"], "not_a_user_path");
    assert_eq!(
        last["metadata"]["simulation"],
        json!({"anonymous": false, "roles": ["Clerk"]})
    );
    // Without a simulation there is no right; roles need state or chain.
    let (_, plain) = all_rows(
        &tools,
        "domain_model",
        json!({"entity": "Order", "state": "Submitted"}),
    )
    .await;
    assert!(plain.iter().all(|r| r.get("right").is_none()), "{plain:#?}");
    assert!(domain(&tools, json!({"roles": ["Clerk"]})).await.is_err());
    let both = json!({"chain": true, "roles": [], "anonymous": true});
    assert!(domain(&tools, both).await.is_err());
}

/// F02: a `[views] backends` path from the configuration must not make the
/// server read a directory outside its root, in `ui_flow` and in `check`.
#[tokio::test]
async fn configured_backends_outside_the_server_root_are_not_read() {
    let base = tempfile::tempdir().unwrap();
    let (root, outside) = (base.path().join("root"), base.path().join("outside"));
    let files = [
        (root.join("Web/Web.csproj"), "<Project />"),
        (root.join("Web/Pages/Home.razor"), "@page \"/\"\n"),
        (root.join("src/a.ts"), "export const a = 1;\n"),
        (
            root.join("archguard.toml"),
            "[views]\nbackends = [\"../outside\"]\nconventions = [\"endpoint_access_declared\"]\n",
        ),
        (outside.join("Api/Api.csproj"), "<Project />"),
        (
            outside.join("Api/SecretController.cs"),
            "[ApiController]\n[Route(\"api/secret\")]\npublic class SecretController : ControllerBase\n{\n    [HttpGet]\n    public IActionResult Get() => Ok();\n}\n",
        ),
    ];
    for (path, text) in files {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let tools = ArchguardTools::new(root);
    let flow = ui_flow(&tools, json!({})).await.unwrap().to_string();
    assert!(!flow.contains("SecretController"), "{flow}");
    assert!(flow.contains("outside the server root"), "{flow}");
    let checked = tools
        .check(Parameters(serde_json::from_value(json!({})).unwrap()))
        .await
        .unwrap();
    assert!(!checked.contains("SecretController"), "{checked}");
}

/// A symlinked configuration inside a backend leaks nothing of its target
/// outside the server root, neither through `ui_flow` nor `service_topology`.
#[cfg(unix)]
#[tokio::test]
async fn a_symlinked_backend_configuration_leaks_no_content_through_the_tools() {
    let base = tempfile::tempdir().unwrap();
    let (root, outside) = (base.path().join("root"), base.path().join("outside"));
    let files = [
        (root.join("Web/Web.csproj"), "<Project />"),
        (root.join("src/a.ts"), "export const a = 1;\n"),
        (
            root.join("archguard.toml"),
            "[views]\nbackends = [\"inside_with_cfglink\"]\n",
        ),
        (
            root.join("inside_with_cfglink/Api/Api.csproj"),
            "<Project />",
        ),
        (outside.join("leak.toml"), "LEAKMARK = = broken\n"),
    ];
    for (path, text) in files {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    std::os::unix::fs::symlink(
        outside.join("leak.toml"),
        root.join("inside_with_cfglink/archguard.toml"),
    )
    .unwrap();
    let tools = ArchguardTools::new(root);
    let flow = ui_flow(&tools, json!({})).await.unwrap().to_string();
    assert!(!flow.contains("LEAKMARK"), "{flow}");
    let params = serde_json::from_value(json!({"path": "inside_with_cfglink"})).unwrap();
    let topology = tools.service_topology(Parameters(params)).await.unwrap();
    assert!(!topology.contains("LEAKMARK"), "{topology}");
}

#[tokio::test]
async fn domain_model_counts_and_lists_controller_only_dtos() {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in [
        (
            "Api/ThingsController.cs",
            "using Microsoft.AspNetCore.Mvc;\nnamespace S;\npublic class ThingsController : Controller\n{\n    public int Get([FromBody] CreateThing c) { return 0; }\n}\n",
        ),
        (
            "Models/CreateThing.cs",
            "namespace S;\npublic class CreateThing { public string Name { get; set; } }\n",
        ),
    ] {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let tools = ArchguardTools::new(dir.path().into());
    let (last, rows) = all_rows(&tools, "domain_model", json!({"query": "creatething"})).await;
    assert_eq!(last["metadata"]["summary"]["api_models"], 1);
    let api = of_kind(&rows, "api_model");
    assert_eq!(api.len(), 1, "{rows:#?}");
    assert_eq!(api[0]["api_model"]["name"], "CreateThing");
    assert_eq!(
        api[0]["api_model"]["reason"],
        "action body of ThingsController.Get"
    );
    assert!(of_kind(&rows, "entity").is_empty());
}
