//! `archguard domain`: rules, validations, integration events and chains
//! across services (ADR 0022, Verlauf 2026-11-20), on an eShop-shaped project.
use serde_json::Value;
use std::{fs, path::Path, process::Command};

const FILES: &[(&str, &str)] = &[
    ("Ordering.API/Ordering.API.csproj", "<Project />"),
    (
        "Ordering.API/Domain/Order.cs",
        r#"public interface IAggregateRoot { }
public enum OrderStatus { Submitted, Paid }
public class Order : IAggregateRoot
{
    public OrderStatus Status { get; private set; }
    public void SetPaid()
    {
        if (Status != OrderStatus.Submitted)
        {
            throw new OrderingDomainException("Only submitted orders can be paid");
        }
        AddDomainEvent(new OrderPaidDomainEvent(Id));
        Status = OrderStatus.Paid;
    }
}
"#,
    ),
    (
        "Ordering.API/Application/Handlers.cs",
        r#"public record OrderPaidIntegrationEvent(int OrderId) : IntegrationEvent;
public class PayOrderCommandHandler : IRequestHandler<PayOrderCommand, bool>
{
    public Task<bool> Handle(PayOrderCommand c, CancellationToken t) { _order.SetPaid(); return Task.FromResult(true); }
}
public class PaymentSucceededHandler(IMediator mediator) : IIntegrationEventHandler<PaymentSucceededIntegrationEvent>
{
    public async Task Handle(PaymentSucceededIntegrationEvent @event) => await mediator.Send(new PayOrderCommand(@event.OrderId));
}
public class OrderPaidDomainEventHandler : INotificationHandler<OrderPaidDomainEvent>
{
    public async Task Handle(OrderPaidDomainEvent e, CancellationToken t)
    {
        var integrationEvent = new OrderPaidIntegrationEvent(e.OrderId);
        await _orderingIntegrationEventService.AddAndSaveEventAsync(integrationEvent);
    }
}
public class PayOrderCommandValidator : AbstractValidator<PayOrderCommand>
{
    public PayOrderCommandValidator() { RuleFor(c => c.OrderId).NotEmpty().WithMessage("No order"); }
}
"#,
    ),
    ("Catalog.API/Catalog.API.csproj", "<Project />"),
    (
        "Catalog.API/Model/CatalogItem.cs",
        r#"public class CatalogItem
{
    public int AvailableStock { get; set; }
    public int RemoveStock(int quantity)
    {
        if (AvailableStock == 0)
        {
            throw new CatalogDomainException($"Empty stock, {quantity} wanted");
        }
        return quantity;
    }
}
public class OrderPaidIntegrationEventHandler : IIntegrationEventHandler<OrderPaidIntegrationEvent>
{
    public Task Handle(OrderPaidIntegrationEvent @event) => Task.CompletedTask;
}
"#,
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

#[test]
fn domain_text_has_rules_validations_and_integration_events() {
    let dir = project();
    let text = run(dir.path(), &["domain"]);
    assert!(
        text.lines().nth(1) == Some("Rules 2 · validations 1 · integration events 1"),
        "{text}"
    );
    assert!(text.contains("Rules (throws when): 2\n  CatalogItem\n    RemoveStock · AvailableStock == 0 → Empty stock, {quantity} wanted · CatalogDomainException · Catalog.API/Model/CatalogItem.cs:8\n  Order\n    SetPaid · Status != OrderStatus.Submitted → Only submitted orders can be paid · OrderingDomainException · Ordering.API/Domain/Order.cs:10\n"), "{text}");
    assert!(
        text.contains("Validations: 1\n  PayOrderCommand.OrderId: NotEmpty(), WithMessage(\"No order\") · Ordering.API/Application/Handlers.cs:20\n"),
        "{text}"
    );
    assert!(
        text.contains("Integration events: 1\n  OrderPaidIntegrationEvent: published by OrderPaidDomainEventHandler.Handle [Ordering.API] → handled by OrderPaidIntegrationEventHandler [Catalog.API]\n"),
        "{text}"
    );
    let order = run(dir.path(), &["domain", "--entity", "Order"]);
    assert!(
        order.contains("Rules (throws when): 1\n  Order\n"),
        "{order}"
    );
    assert!(order.contains("Integration events: 1\n"), "{order}");
    assert!(order.contains("Validations: 0\n"), "{order}");
}

#[test]
fn domain_json_and_chain_across_services() {
    let dir = project();
    let value = json(dir.path(), &["domain"]);
    assert_eq!(value["summary"]["rules"], 2);
    assert_eq!(
        value["model"]["rules"][1]["condition"],
        "Status != OrderStatus.Submitted"
    );
    assert_eq!(value["model"]["validations"][0]["property"], "OrderId");
    assert_eq!(
        value["model"]["integration_events"][0]["handled_by"],
        serde_json::json!(["OrderPaidIntegrationEventHandler"])
    );
    let text = run(
        dir.path(),
        &["domain", "--entity", "Order", "--state", "Paid"],
    );
    assert!(
        text.contains("  ⇒ OrderPaidIntegrationEvent published by OrderPaidDomainEventHandler.Handle [Ordering.API] → OrderPaidIntegrationEventHandler [Catalog.API] → no use case found · Catalog.API/Model/CatalogItem.cs:13\n"),
        "{text}"
    );
    let chains = json(
        dir.path(),
        &["domain", "--entity", "Order", "--state", "Paid"],
    );
    let next = &chains["chains"][0]["continues"][0];
    assert_eq!(next["via"]["by"], "OrderPaidDomainEventHandler.Handle");
    assert_eq!(next["message_handler"]["service"], "Catalog.API");
}
