//! Messaging in service flows (ADR 0006): where a service's source
//! publishes to a topic or queue (`producer.send({topic: 'x'})`,
//! `kafkaTemplate.send("x")`, `sendToQueue('x')`, `basic_publish(…,
//! routing_key='x')`, `nc.publish('x')`) and where it consumes one
//! (`consumer.subscribe({topic: 'x'})`, `@KafkaListener(topics = "x")`,
//! `@RabbitListener(queues = "x")`, `KafkaConsumer('x')`,
//! `basic_consume(queue='x')`, `channel.consume('x')`). A consumer becomes
//! a route with method `EVENT` and the topic as path; a publication
//! becomes an `EVENT` step to every service consuming that topic, or to
//! `topic:<name>` when no consumer is in the source. Names must match
//! literally: exchanges bound to differently named queues stay apart.

use archguard_core::{RouteStep, ServiceRoute};
use std::collections::BTreeMap;

pub(crate) const EVENT: &str = "EVENT";

const PUBLISH: [&str; 8] = [
    "producer.send(",
    "producer.produce(",
    "kafkaTemplate.send(",
    ".convertAndSend(",
    "sendToQueue(",
    "basic_publish(",
    ".publish(",
    "publisher.send(",
];

const CONSUME: [&str; 8] = [
    "consumer.subscribe(",
    "@KafkaListener(",
    "@RabbitListener(",
    "KafkaConsumer(",
    "basic_consume(",
    ".consume(",
    "nc.subscribe(",
    "@JmsListener(",
];

/// Argument names that carry the topic or queue.
const KEYS: [&str; 12] = [
    "topic:",
    "topics:",
    "topic=",
    "topics =",
    "topics=",
    "queues =",
    "queues=",
    "queue=",
    "queue:",
    "routing_key=",
    "subject:",
    "destination =",
];

/// Topics published in `segment`, with their byte offset.
pub(crate) fn published(segment: &str) -> Vec<(String, usize)> {
    found(segment, &PUBLISH)
}

/// Topics consumed in `text`; each starts a handler at its offset.
pub(crate) fn consumed(text: &str) -> Vec<(String, usize)> {
    found(text, &CONSUME)
}

fn found(text: &str, markers: &[&str]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = markers
        .iter()
        .flat_map(|m| {
            text.match_indices(m)
                .filter_map(|(at, _)| topic(&text[at + m.len()..]).map(|t| (t, at)))
        })
        .collect();
    out.sort_by_key(|(_, at)| *at);
    out.dedup_by_key(|(_, at)| *at);
    out
}

/// The topic of the argument list starting `args`: a leading literal or
/// one named by [`KEYS`] before the closing parenthesis.
fn topic(args: &str) -> Option<String> {
    let args = cut(args, 200);
    let args = &args[..args.find(')').unwrap_or(args.len())];
    let lead = args
        .trim_start()
        .trim_start_matches(['{', '['])
        .trim_start();
    if let Some(name) = quoted(lead) {
        return Some(name);
    }
    KEYS.iter().find_map(|k| {
        let at = args.find(k)?;
        quoted(
            args[at + k.len()..]
                .trim_start()
                .trim_start_matches('[')
                .trim_start(),
        )
    })
}

/// The non-empty literal at the start of `text`.
fn quoted(text: &str) -> Option<String> {
    let quote = text
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let body = &text[1..];
    let name = &body[..body.find(quote)?];
    let plain = !name.is_empty()
        && !name.contains("${")
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "._-/:#*>".contains(c));
    plain.then(|| name.to_string())
}

fn cut(text: &str, n: usize) -> &str {
    let mut end = n.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// An unresolved publication of `topic` at `line`.
pub(crate) fn step(topic: String) -> RouteStep {
    RouteStep {
        to: String::new(),
        method: EVENT.into(),
        path: topic,
        line: 0,
        parallel: false,
    }
}

/// Points each publication at the services consuming its topic.
pub(crate) fn connect(routes: &mut [ServiceRoute]) {
    let mut consumers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in routes.iter().filter(|r| r.method == EVENT) {
        let list = consumers.entry(r.path.clone()).or_default();
        if !list.contains(&r.service) {
            list.push(r.service.clone());
        }
    }
    for r in routes.iter_mut() {
        let steps = std::mem::take(&mut r.steps);
        for s in steps {
            if s.method != EVENT || !s.to.is_empty() {
                r.steps.push(s);
                continue;
            }
            match consumers.get(&s.path) {
                Some(services) => r.steps.extend(services.iter().map(|to| RouteStep {
                    to: to.clone(),
                    ..s.clone()
                })),
                None => r.steps.push(RouteStep {
                    to: format!("topic:{}", s.path),
                    ..s
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publications_across_clients() {
        let text = r#"
await producer.send({ topic: 'order.placed', messages: [{ value }] });
kafkaTemplate.send("invoice.created", payload);
channel.sendToQueue('mail', Buffer.from(x));
ch.basic_publish(exchange='', routing_key='audit', body=b)
nc.publish(`stock.low`, data);
obs.publish(() => console.log('not a topic'));
producer.send({ topic: `orders.${region}` });
"#;
        let topics: Vec<String> = published(text).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            topics,
            [
                "order.placed",
                "invoice.created",
                "mail",
                "audit",
                "stock.low"
            ],
            "lambdas and built topic names do not count"
        );
    }

    #[test]
    fn consumers_across_clients() {
        let text = r#"
await consumer.subscribe({ topics: ['order.placed'], fromBeginning: true });
@KafkaListener(topics = "invoice.created", groupId = "x")
@RabbitListener(queues = "mail")
consumer = KafkaConsumer('audit', bootstrap_servers=s)
ch.basic_consume(queue='stock.low', on_message_callback=cb)
source$.subscribe(v => handle('x'));
"#;
        let topics: Vec<String> = consumed(text).into_iter().map(|(t, _)| t).collect();
        assert_eq!(
            topics,
            [
                "order.placed",
                "invoice.created",
                "mail",
                "audit",
                "stock.low"
            ]
        );
    }

    #[test]
    fn publications_reach_every_consumer_or_their_topic() {
        let route = |service: &str, method: &str, path: &str, steps: Vec<RouteStep>| ServiceRoute {
            service: service.into(),
            method: method.into(),
            path: path.into(),
            file: "f".into(),
            line: 1,
            steps,
            failures: vec![],
            catches: false,
        };
        let mut routes = vec![
            route(
                "orders",
                "POST",
                "/orders",
                vec![step("order.placed".into()), step("audit".into())],
            ),
            route("shipping", EVENT, "order.placed", vec![]),
            route("billing", EVENT, "order.placed", vec![]),
        ];
        connect(&mut routes);
        let to: Vec<&str> = routes[0].steps.iter().map(|s| s.to.as_str()).collect();
        assert_eq!(to, ["shipping", "billing", "topic:audit"]);
    }
}
