use archguard_engine::analyze_services;
use std::fs;
fn write(root: &std::path::Path, name: &str, text: &str) {
    let p = root.join(name);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
#[test]
fn aspire_communication_is_evidenced_and_separate_from_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let r = temp.path();
    for p in ["Host", "Web", "Orders", "Stock"] {
        write(
            r,
            &format!("src/{p}/{p}.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>",
        );
    }
    write(
        r,
        "src/Host/AppHost.cs",
        r#"
var bus = builder.AddRabbitMQ("eventbus");
var web = builder.AddDotnetProject("web", "../Web").WithReference(orders);
var orders = builder.AddDotnetProject("orders", "../Orders").WithReference(bus);
var stock = builder.AddDotnetProject("stock", "../Stock").WithReference(bus);
"#,
    );
    write(
        r,
        "src/Web/Clients.cs",
        r#"
builder.Services.AddHttpClient<OrdersClient>(o=>o.BaseAddress=new("https+http://orders"));
builder.Services.AddGrpcClient<StockClient>(o=>o.Address=new("http://stock"));
// builder.Services.AddHttpClient<Fake>(o=>o.BaseAddress=new("http://web"));
var documentation = "https://web/password?token=secret";
"#,
    );
    write(
        r,
        "src/Orders/Events.cs",
        r#"
builder.AddRabbitMqEventBus("eventbus").AddSubscription<StockConfirmedIntegrationEvent, Handler>();
var message = new ValidateStockIntegrationEvent(1);
await eventBus.PublishAsync(message);
var unused = new NeverPublishedIntegrationEvent();
"#,
    );
    write(
        r,
        "src/Stock/Events.cs",
        r#"
builder.AddRabbitMqEventBus("eventbus").AddSubscription<ValidateStockIntegrationEvent, Handler>();
var message = new StockConfirmedIntegrationEvent(1);
await eventBus.PublishAsync(message);
"#,
    );
    let report = serde_json::to_value(analyze_services(r).unwrap()).unwrap();
    let edges = report["dependencies"].as_array().unwrap();
    for kind in ["http_call", "grpc_call", "event", "configured_reference"] {
        assert!(
            edges.iter().any(|e| e["kind"] == kind),
            "missing {kind}: {report}"
        );
    }
    assert_eq!(edges.iter().filter(|e| e["kind"] == "event").count(), 2);
    assert!(
        report["cycles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "event" && c["nodes"].as_array().unwrap().len() == 2)
    );
    assert!(!report.to_string().contains("secret"));
    assert!(
        edges
            .iter()
            .all(|e| !e["evidence"].as_array().unwrap().is_empty())
    );
}
#[test]
fn names_alone_and_comments_do_not_establish_events_or_services() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    write(r, "A/A.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>");
    write(
        r,
        "A/Test.cs",
        r#"// builder.AddDotnetProject("fake", "../A");
class SameIntegrationEvent {} // PublishAsync(new SameIntegrationEvent());
"#,
    );
    assert!(analyze_services(r).unwrap().dependencies.is_empty());
}

#[test]
fn different_brokers_and_non_destination_urls_do_not_create_communication() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    for p in ["Host", "A", "B"] {
        write(
            r,
            &format!("{p}/{p}.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>",
        );
    }
    write(
        r,
        "Host/AppHost.cs",
        r#"
var first=builder.AddRabbitMQ("first");var second=builder.AddRabbitMQ("second");
var a=builder.AddDotnetProject("a","../A").WithReference(first);
var b=builder.AddDotnetProject("b","../B").WithReference(second);
"#,
    );
    write(
        r,
        "A/A.cs",
        r#"
builder.AddRabbitMqEventBus("first");
await eventBus.PublishAsync(new SharedIntegrationEvent());
builder.Services.AddHttpClient<Client>(o=>o.DefaultRequestHeaders.Add("help","http://b"));
"#,
    );
    write(
        r,
        "B/B.cs",
        r#"builder.AddRabbitMqEventBus("second").AddSubscription<SharedIntegrationEvent,Handler>();"#,
    );
    let report = analyze_services(r).unwrap();
    assert_eq!(
        report.dependencies.len(),
        2,
        "only two declared broker references"
    );
    assert!(
        report
            .dependencies
            .iter()
            .all(|e| e.kind == archguard_core::ServiceDependencyKind::ConfiguredReference)
    );
}

#[test]
fn an_unused_event_in_another_method_is_not_a_publisher() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    for p in ["Host", "A", "B"] {
        write(
            r,
            &format!("{p}/{p}.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>",
        );
    }
    write(
        r,
        "Host/AppHost.cs",
        r#"
var bus=builder.AddRabbitMQ("bus");
var a=builder.AddDotnetProject("a","../A").WithReference(bus);
var b=builder.AddDotnetProject("b","../B").WithReference(bus);
"#,
    );
    write(
        r,
        "A/A.cs",
        r#"
builder.AddRabbitMqEventBus("bus");
class Handler {
void Unused(){var message=new GhostIntegrationEvent();}
async Task Publish(object message){await bus.PublishAsync(message);}
async Task Overwritten(){var message=new GhostIntegrationEvent();message=GetUnknown();await bus.PublishAsync(message);}
}
"#,
    );
    write(
        r,
        "B/B.cs",
        r#"builder.AddRabbitMqEventBus("bus").AddSubscription<GhostIntegrationEvent,Handler>();"#,
    );
    let report = analyze_services(r).unwrap();
    assert_eq!(
        report.dependencies.len(),
        2,
        "only two declared broker references"
    );
    assert!(
        report
            .dependencies
            .iter()
            .all(|e| e.kind == archguard_core::ServiceDependencyKind::ConfiguredReference)
    );
}

#[test]
fn aspire_resources_completion_and_constant_uri_are_connected() {
    let t = tempfile::tempdir().unwrap();
    let r = t.path();
    for p in ["Host", "Api", "Worker", "Web"] {
        write(
            r,
            &format!("{p}/{p}.csproj"),
            "<Project Sdk=\"Microsoft.NET.Sdk.Web\"/>",
        );
    }
    write(
        r,
        "Host/AppHost.cs",
        r#"
var sql=builder.AddSqlServer("sql");
var db=sql.AddDatabase("data");
var worker=builder.AddProject<Projects.Worker>("worker").WithReference(db).WaitFor(db);
var api=builder.AddProject<Projects.Api>("api").WithReference(db).WaitForCompletion(worker);
var web=builder.AddProject<Projects.Web>("web").WithReference(api);
"#,
    );
    write(
        r,
        "Web/Program.cs",
        r#"
var endpoint = new Uri("http://api");
builder.Services.AddHttpClient<ApiClient>(client => client.BaseAddress = endpoint);
"#,
    );
    let r = analyze_services(r).unwrap();
    let id = |n: &str| r.services.iter().find(|s| s.name == n).unwrap().id.clone();
    assert!(r.dependencies.iter().any(|e| e.from == id("api")
        && e.to == id("worker")
        && e.kind == archguard_core::ServiceDependencyKind::Startup));
    assert!(
        r.dependencies
            .iter()
            .any(|e| e.from == id("api") && e.to == id("data"))
    );
    assert!(r.dependencies.iter().any(|e| e.from == id("web")
        && e.to == id("api")
        && e.kind == archguard_core::ServiceDependencyKind::HttpCall));
}
