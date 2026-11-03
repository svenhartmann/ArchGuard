//! Literal Spring configuration and client evidence. No application/profile is executed.
use super::service_communications::{Token, arguments, is, lex, scopes};
use super::support::{diagnostic, relative};
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceEvidence, ServiceTopology,
};
use serde::Deserialize;
use serde_yaml_ng::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
#[derive(Clone)]
struct Input {
    file: String,
    text: String,
}
fn inputs(root: &Path, dir: &Path, out: &mut Vec<Input>, depth: usize, budget: &mut usize) {
    if depth > 20 || out.len() >= 2000 || *budget > 32_000_000 {
        return;
    }
    if fs::canonicalize(dir).map_or(true, |p| !p.starts_with(root))
        || fs::symlink_metadata(dir).map_or(true, |m| m.file_type().is_symlink())
    {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().take(2001).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        if out.len() >= 2000 || *budget > 32_000_000 {
            break;
        }
        let Ok(ty) = entry.file_type() else {
            continue;
        };
        if ty.is_symlink() {
            continue;
        }
        let p = entry.path();
        if ty.is_dir() {
            inputs(root, &p, out, depth + 1, budget);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".java")
            || [".yml", ".yaml", ".properties"]
                .iter()
                .any(|e| name.ends_with(e)))
        {
            continue;
        }
        if entry.metadata().map_or(true, |m| m.len() > 1_048_576) {
            continue;
        }
        if let Ok(text) = fs::read_to_string(&p) {
            *budget += text.len();
            out.push(Input {
                file: relative(root, &p),
                text,
            });
        }
    }
}
fn flatten(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::Mapping(m) => {
            for (k, v) in m {
                if let Some(k) = k.as_str() {
                    flatten(
                        v,
                        &if path.is_empty() {
                            k.into()
                        } else {
                            format!("{path}.{k}")
                        },
                        out,
                    )
                }
            }
        }
        Value::Sequence(a) => {
            for (i, v) in a.iter().enumerate() {
                flatten(v, &format!("{path}.{i}"), out)
            }
        }
        Value::String(s) => out.push((path.into(), s.clone())),
        _ => {}
    }
}
fn settings(input: &Input) -> Vec<(String, String)> {
    if input.file.ends_with(".properties") {
        return input
            .text
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.starts_with(['#', '!']) {
                    return None;
                }
                let (k, v) = line.split_once('=').or_else(|| line.split_once(':'))?;
                Some((k.trim().into(), v.trim().into()))
            })
            .collect();
    }
    let mut out = Vec::new();
    for doc in serde_yaml_ng::Deserializer::from_str(&input.text).take(2000) {
        if let Ok(v) = Value::deserialize(doc) {
            flatten(&v, "", &mut out);
        } else {
            break;
        }
    }
    out
}
fn host(value: &str) -> Option<String> {
    if value.contains("${") || value.contains('\\') {
        return None;
    }
    let (scheme, rest) = value.split_once("://")?;
    if !["lb", "http", "https"].contains(&scheme) {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let hostname = authority.split(':').next()?;
    if hostname.is_empty()
        || !hostname
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
    {
        return None;
    }
    Some(hostname.to_lowercase())
}
fn proof(input: &Input, pointer: &str, line: Option<u32>, description: &str) -> ServiceEvidence {
    ServiceEvidence {
        file: input.file.clone(),
        pointer: pointer.into(),
        line,
        description: description.into(),
    }
}
fn insert(
    edges: &mut BTreeMap<(String, String, Kind), BTreeSet<ServiceEvidence>>,
    from: &str,
    to: &str,
    kind: Kind,
    evidence: ServiceEvidence,
) {
    if from != to {
        edges
            .entry((from.into(), to.into(), kind))
            .or_default()
            .insert(evidence);
    }
}
fn unique(map: &BTreeMap<String, Vec<String>>, name: &str) -> Option<String> {
    let ids = map.get(&name.to_lowercase())?;
    (ids.len() == 1).then(|| ids[0].clone())
}
fn local_configuration(file: &str) -> bool {
    let name = Path::new(file)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    !name.ends_with(".java")
        && (name.starts_with("application.")
            || name.starts_with("application-")
            || name.starts_with("bootstrap.")
            || name.starts_with("bootstrap-"))
}

// Recognize a deliberately small, side-effect-free method grammar. Any extra
// statement, overload, mutation, conditional or other expression stays unresolved.
fn parameter_helpers(tokens: &[Token]) -> BTreeMap<String, String> {
    let mut helpers = BTreeMap::new();
    for window in tokens.windows(13) {
        if is(&window[0], "String")
            && is(&window[2], "(")
            && is(&window[3], "String")
            && is(&window[5], ")")
            && is(&window[6], "{")
            && is(&window[7], "return")
            && window[8].string
            && matches!(window[8].text.as_str(), "http://" | "https://")
            && is(&window[9], "+")
            && window[10].text == window[4].text
            && !window[10].string
            && is(&window[11], ";")
            && is(&window[12], "}")
        {
            let name = &window[1].text;
            let declarations = tokens
                .windows(3)
                .filter(|w| is(&w[0], "String") && is(&w[1], name) && is(&w[2], "("))
                .count();
            if declarations == 1 {
                helpers.insert(name.clone(), window[8].text.clone());
            }
        }
    }
    helpers
}
fn literal_expression(tokens: &[Token], helpers: &BTreeMap<String, String>) -> Option<String> {
    let first = tokens.first()?;
    if first.string {
        return Some(first.text.clone());
    }
    let prefix = helpers.get(&first.text)?;
    if tokens.len() >= 4 && is(&tokens[1], "(") && tokens[2].string && is(&tokens[3], ")") {
        let name = &tokens[2].text;
        if !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_'))
        {
            return Some(format!("{prefix}{name}"));
        }
    }
    None
}
struct MessageEndpoint {
    owner: String,
    protocol: &'static str,
    broker: String,
    channel: String,
    evidence: ServiceEvidence,
}
fn comma_arguments(tokens: &[Token]) -> Vec<&[Token]> {
    let (mut start, mut depth) = (0, 0i32);
    let mut result = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if ["(", "{", "["].iter().any(|v| is(t, v)) {
            depth += 1;
        }
        if [")", "}", "]"].iter().any(|v| is(t, v)) {
            depth -= 1;
        }
        if depth == 0 && is(t, ",") {
            result.push(&tokens[start..i]);
            start = i + 1;
        }
    }
    result.push(&tokens[start..]);
    result
}
fn broker_configuration(files: &[Input], protocol: &str) -> Option<String> {
    let key = if protocol == "kafka" {
        "spring.kafka.bootstrap-servers"
    } else {
        "spring.rabbitmq.addresses"
    };
    let values: BTreeSet<_> = files
        .iter()
        .filter(|i| !i.file.ends_with(".java"))
        .flat_map(settings)
        .filter(|(k, _)| k == key || protocol == "rabbit" && k == "spring.rabbitmq.host")
        .map(|(_, v)| v)
        .collect();
    if values.len() != 1 {
        return None;
    }
    let value = values.first()?;
    if value.is_empty() || value.contains(['$', '@', '/', '\\']) {
        return None;
    }
    let mut hosts: Vec<_> = value.split(',').map(str::trim).collect();
    hosts.sort_unstable();
    let mut identity = hosts.join(",");
    if protocol == "rabbit" {
        for key in ["spring.rabbitmq.port", "spring.rabbitmq.virtual-host"] {
            let settings: BTreeSet<_> = files
                .iter()
                .filter(|i| !i.file.ends_with(".java"))
                .flat_map(settings)
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v)
                .collect();
            if settings.len() > 1 || settings.iter().any(|v| v.contains('$')) {
                return None;
            }
            identity.push('|');
            identity.push_str(
                settings
                    .first()
                    .map(String::as_str)
                    .unwrap_or(if key.ends_with("port") { "5672" } else { "/" }),
            );
        }
    }
    Some(identity)
}
fn collect_messaging(
    input: &Input,
    tokens: &[Token],
    files: &[Input],
    owner: &str,
    producers: &mut Vec<MessageEndpoint>,
    consumers: &mut Vec<MessageEndpoint>,
) {
    for (protocol, template, annotation, method, attribute) in [
        ("kafka", "KafkaTemplate", "KafkaListener", "send", "topics"),
        (
            "rabbit",
            "RabbitTemplate",
            "RabbitListener",
            "convertAndSend",
            "queues",
        ),
    ] {
        let package = if protocol == "kafka" {
            "org.springframework.kafka."
        } else {
            "org.springframework.amqp."
        };
        // Tokenized imports exclude comment/string lookalikes.
        let imported = |name: &str| {
            tokens.split(|t| is(t, ";")).any(|statement| {
                let Some(at) = statement.iter().position(|t| is(t, "import")) else {
                    return false;
                };
                let path = statement[at + 1..]
                    .iter()
                    .map(|t| t.text.as_str())
                    .collect::<String>();
                path.starts_with(package) && path.ends_with(name)
            })
        };
        if !imported(template) && !imported(annotation) {
            continue;
        }
        let Some(broker) = broker_configuration(files, protocol) else {
            continue;
        };
        let mut receivers = BTreeSet::new();
        if imported(template) {
            for (i, _) in tokens.iter().enumerate().filter(|(_, t)| is(t, template)) {
                let mut j = i + 1;
                if tokens.get(j).is_some_and(|t| is(t, "<")) {
                    let mut depth = 1;
                    j += 1;
                    while j < tokens.len() && depth > 0 {
                        if is(&tokens[j], "<") {
                            depth += 1;
                        }
                        if is(&tokens[j], ">") {
                            depth -= 1;
                        }
                        j += 1;
                    }
                }
                if let Some(name) = tokens.get(j)
                    && tokens
                        .get(j + 1)
                        .is_some_and(|t| is(t, ";") || is(t, "=") || is(t, ",") || is(t, ")"))
                {
                    receivers.insert(name.text.clone());
                }
            }
        }
        for (i, t) in tokens.iter().enumerate() {
            let channel = if is(t, method)
                && i >= 2
                && is(&tokens[i - 1], ".")
                && receivers.contains(&tokens[i - 2].text)
            {
                let Some(args) = arguments(tokens, i) else {
                    continue;
                };
                let parts = comma_arguments(args);
                let arg = if protocol == "rabbit" {
                    if parts.len() == 2 {
                        parts[0]
                    } else if parts.len() == 3
                        && parts[0].len() == 1
                        && parts[0][0].string
                        && parts[0][0].text.is_empty()
                    {
                        parts[1]
                    } else {
                        continue;
                    }
                } else {
                    if parts.len() < 2 {
                        continue;
                    }
                    parts[0]
                };
                if arg.len() != 1 || !arg[0].string {
                    continue;
                }
                Some((&arg[0], true))
            } else if imported(annotation)
                && (is(t, &format!("@{annotation}"))
                    || is(t, annotation) && i > 0 && is(&tokens[i - 1], "@"))
            {
                let Some(args) = arguments(tokens, i) else {
                    continue;
                };
                comma_arguments(args)
                    .into_iter()
                    .find(|a| a.len() == 3 && is(&a[0], attribute) && is(&a[1], "=") && a[2].string)
                    .map(|a| (&a[2], false))
            } else {
                None
            };
            let Some((channel, publish)) = channel else {
                continue;
            };
            if producers.len() + consumers.len() >= 10_000 {
                return;
            }
            if channel.text.is_empty() || channel.text.contains(['$', '#', '*']) {
                continue;
            }
            let endpoint = MessageEndpoint {
                owner: owner.into(),
                protocol,
                broker: broker.clone(),
                channel: channel.text.clone(),
                evidence: proof(
                    input,
                    if publish {
                        "spring-message-producer"
                    } else {
                        "spring-message-listener"
                    },
                    Some(t.line),
                    "Static inferred messaging candidate: literal topic/queue and literal broker configuration match. Publication, delivery and active profiles are not proven.",
                ),
            };
            if publish {
                producers.push(endpoint);
            } else {
                consumers.push(endpoint);
            }
        }
    }
}

fn service_scope(report: &ServiceTopology, id: &str) -> String {
    report
        .services
        .iter()
        .find(|s| s.id == id)
        .map(|s| {
            s.networks
                .iter()
                .find(|n| n.starts_with("kubernetes:"))
                .cloned()
                .unwrap_or_else(|| s.compose_file.clone())
        })
        .unwrap_or_default()
}
pub(super) fn augment(root: &Path, report: &mut ServiceTopology) {
    let projects: Vec<_> = report
        .projects
        .iter()
        .filter(|p| matches!(p.ecosystem.as_str(), "maven" | "gradle") && p.kind != "workspace")
        .cloned()
        .collect();
    if projects.is_empty() {
        return;
    }
    let mut files = BTreeMap::new();
    let mut names = BTreeMap::new();
    let mut budget = 0;
    for project in &projects {
        let mut source = Vec::new();
        inputs(
            root,
            &root.join(&project.path).join("src/main"),
            &mut source,
            0,
            &mut budget,
        );
        let found: BTreeSet<_> = source
            .iter()
            .filter(|s| local_configuration(&s.file))
            .flat_map(settings)
            .filter(|(k, _)| k == "spring.application.name")
            .map(|(_, v)| v)
            .filter(|v| !v.contains("${"))
            .collect();
        if found.len() == 1 {
            names.insert(project.id.clone(), found.into_iter().next().unwrap());
        } else if found.len() > 1 {
            diagnostic(
                report,
                &project.manifest,
                None,
                "Multiple Spring application names across profiles; service identity left unresolved.",
            );
        }
        files.insert(project.id.clone(), source);
    }
    // Compose image identity supplies a project mapping only when the artifact name matches exactly.
    let mut images = BTreeMap::<String, BTreeSet<String>>::new();
    for file in &report.compose_files {
        if fs::symlink_metadata(root.join(file))
            .map_or(true, |m| m.file_type().is_symlink() || m.len() > 1_048_576)
        {
            continue;
        }
        if let Ok(text) = fs::read_to_string(root.join(file))
            && let Ok(doc) = serde_yaml_ng::from_str::<Value>(&text)
            && let Some(services) = doc.get("services").and_then(Value::as_mapping)
        {
            for (name, service) in services {
                if let (Some(name), Some(image)) =
                    (name.as_str(), service.get("image").and_then(Value::as_str))
                {
                    let artifact = image
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .split([':', '@'])
                        .next()
                        .unwrap_or("");
                    images
                        .entry(artifact.into())
                        .or_default()
                        .insert(name.into());
                }
            }
        }
    }
    for project in &projects {
        if !names.contains_key(&project.id)
            && let Some(matches) = images.get(&project.name)
            && matches.len() == 1
        {
            names.insert(project.id.clone(), matches.first().unwrap().clone());
        }
    }
    let mut owners = BTreeMap::new();
    for project in &projects {
        let Some(name) = names.get(&project.id) else {
            continue;
        };
        if names.values().filter(|v| *v == name).count() != 1 {
            diagnostic(
                report,
                &project.manifest,
                None,
                "Duplicate Spring service identity; project mapping omitted.",
            );
            continue;
        }
        let existing: Vec<_> = report
            .services
            .iter()
            .filter(|s| s.name == *name && !s.compose_file.is_empty())
            .map(|s| s.id.clone())
            .collect();
        let ids = if existing.is_empty() {
            vec![format!("spring:{}", project.id)]
        } else {
            existing
        };
        let mut mapped = Vec::new();
        for id in ids {
            if let Some(service) = report.services.iter_mut().find(|s| s.id == id) {
                if service.project.is_some() && service.project.as_ref() != Some(&project.id) {
                    continue;
                }
                service.project = Some(project.id.clone());
            } else {
                report.services.push(Service {
                    id: id.clone(),
                    name: name.clone(),
                    compose_file: String::new(),
                    build_context: Some(project.path.clone()),
                    networks: vec![],
                    profiles: vec![],
                    project: Some(project.id.clone()),
                });
            }
            mapped.push(id);
        }
        report.services.retain(|s| s.id != project.id);
        if let Some(p) = report.projects.iter_mut().find(|p| p.id == project.id) {
            p.kind = "application".into();
            p.runtime = Some("spring".into());
        }
        owners.insert(project.id.clone(), mapped);
    }
    // Only explicitly configured classpath native repositories are considered.
    // Filename application identity is the Spring Config naming contract, not a directory guess.
    let mut shared = BTreeMap::<String, Vec<Input>>::new();
    for project in &projects {
        let prefix = if project.path == "." {
            String::new()
        } else {
            format!("{}/", project.path)
        };
        let resources = format!("{prefix}src/main/resources/");
        for input in files[&project.id]
            .iter()
            .filter(|i| local_configuration(&i.file))
        {
            for (key, value) in settings(input) {
                if !matches!(
                    key.as_str(),
                    "spring.cloud.config.server.native.search-locations"
                        | "spring.cloud.config.server.native.searchLocations"
                ) {
                    continue;
                }
                for location in value.split(',') {
                    let Some(location) = location.trim().strip_prefix("classpath:") else {
                        continue;
                    };
                    let location = location.trim_matches('/');
                    if location.contains("..") || location.contains(['$', '*', '\\']) {
                        continue;
                    }
                    let directory = format!("{resources}{location}/");
                    for candidate in &files[&project.id] {
                        let Some(filename) = candidate.file.strip_prefix(&directory) else {
                            continue;
                        };
                        if filename.contains('/') || filename.ends_with(".java") {
                            continue;
                        }
                        let stem = Path::new(filename)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("");
                        let matches: Vec<_> = names
                            .iter()
                            .filter(|(_, name)| {
                                stem == *name
                                    || stem
                                        .strip_prefix(name.as_str())
                                        .is_some_and(|tail| tail.starts_with('-'))
                            })
                            .collect();
                        if matches.len() == 1 {
                            shared
                                .entry(matches[0].0.clone())
                                .or_default()
                                .push(candidate.clone());
                        }
                    }
                }
            }
        }
    }
    for (id, source) in &mut files {
        source.retain(|i| i.file.ends_with(".java") || local_configuration(&i.file));
        source.extend(shared.remove(id).unwrap_or_default());
    }
    let mut edges = BTreeMap::new();
    let mut publications = Vec::new();
    let mut subscriptions = Vec::new();
    for project in &projects {
        let Some(owner_ids) = owners.get(&project.id) else {
            continue;
        };
        for from in owner_ids {
            let scope = service_scope(report, from);
            let mut targets = BTreeMap::<String, Vec<String>>::new();
            for service in &report.services {
                if scope.is_empty() || service_scope(report, &service.id) == scope {
                    targets
                        .entry(service.name.to_lowercase())
                        .or_default()
                        .push(service.id.clone());
                }
            }
            if scope.is_empty() {
                for ids in targets.values_mut() {
                    let deployed: Vec<_> = ids
                        .iter()
                        .filter(|id| {
                            report
                                .services
                                .iter()
                                .any(|s| s.id == **id && !s.compose_file.is_empty())
                        })
                        .cloned()
                        .collect();
                    if !deployed.is_empty() {
                        *ids = deployed;
                    }
                }
            }
            for input in &files[&project.id] {
                if !input.file.ends_with(".java") {
                    for (key, value) in settings(input) {
                        let zuul = key.starts_with("zuul.routes.")
                            && (key.ends_with(".url")
                                || key.ends_with(".serviceId")
                                || key.ends_with(".service-id"));
                        let route = zuul
                            || key.starts_with("spring.cloud.gateway.")
                                && key.contains(".routes.")
                                && key.ends_with(".uri");
                        let reference = key == "spring.config.import"
                            || key == "spring.cloud.config.uri"
                            || key.to_lowercase().starts_with("eureka.client.serviceurl.");
                        if key == "spring.cloud.config.server.git.uri" {
                            diagnostic(
                                report,
                                &input.file,
                                Some(from),
                                "External Spring Config repository is not fetched; remote properties are not covered.",
                            );
                        }
                        if !route && !reference {
                            continue;
                        }
                        if value.contains("${") {
                            diagnostic(
                                report,
                                &input.file,
                                Some(from),
                                "Dynamic Spring endpoint/property is not expanded; runtime configuration may add dependencies.",
                            );
                            continue;
                        }
                        for value in value.split(',') {
                            let value = value
                                .trim()
                                .trim_start_matches("optional:")
                                .trim_start_matches("configserver:");
                            if let Some(name) = host(value).or_else(|| {
                                (zuul && !key.ends_with(".url") && !value.contains(['/', '$']))
                                    .then(|| value.to_lowercase())
                            }) && let Some(to) = unique(&targets, &name)
                            {
                                insert(
                                    &mut edges,
                                    from,
                                    &to,
                                    if route {
                                        Kind::ConfiguredHttp
                                    } else {
                                        Kind::ConfiguredReference
                                    },
                                    proof(
                                        input,
                                        &key,
                                        None,
                                        if route {
                                            "Literal Spring Gateway/Zuul route. Union of declared profiles; not evidence of a runtime request. URL path and query omitted."
                                        } else {
                                            "Spring configuration/discovery-server reference; not an application API call. Profiles are not evaluated."
                                        },
                                    ),
                                );
                            }
                        }
                    }
                    continue;
                }
                let tokens = lex(&input.text);
                collect_messaging(
                    input,
                    &tokens,
                    &files[&project.id],
                    from,
                    &mut publications,
                    &mut subscriptions,
                );
                let parameter_helpers = parameter_helpers(&tokens);
                let token_scope = scopes(&tokens);
                let spring_client = input.text.contains("import org.springframework.web.")
                    && tokens.iter().any(|t| {
                        ["WebClient", "RestClient", "RestTemplate"]
                            .iter()
                            .any(|n| is(t, n))
                    });
                let mut clients = BTreeSet::new();
                let mut literals = BTreeMap::<String, Option<String>>::new();
                let mut helpers = BTreeMap::new();
                for (i, t) in tokens.iter().enumerate() {
                    if ["WebClient", "RestClient", "RestTemplate"]
                        .iter()
                        .any(|n| is(t, n))
                    {
                        let j = if tokens.get(i + 2).is_some_and(|t| is(t, "Builder")) {
                            i + 3
                        } else {
                            i + 1
                        };
                        if let Some(name) = tokens.get(j)
                            && !tokens.get(j + 1).is_some_and(|t| is(t, "("))
                        {
                            clients.insert(name.text.clone());
                        }
                    }
                    if is(t, "=") && i > 0 {
                        let key = tokens[i - 1].text.clone();
                        let value = literal_expression(&tokens[i + 1..], &parameter_helpers);
                        literals
                            .entry(key)
                            .and_modify(|v| *v = None)
                            .or_insert(value);
                    }
                    if is(t, "getInstances")
                        && let Some(arg) = arguments(&tokens, i)
                            .and_then(|a| a.first())
                            .filter(|t| t.string)
                    {
                        let scope = token_scope[i].0;
                        if scope >= 3 && is(&tokens[scope - 1], ")") && is(&tokens[scope - 2], "(")
                        {
                            helpers.insert(tokens[scope - 3].text.clone(), arg.text.clone());
                        }
                    }
                }
                if spring_client {
                    let mut offset = 0;
                    for stmt in tokens.split(|t| is(t, ";")) {
                        for (i, t) in stmt.iter().enumerate() {
                            if ![
                                "uri",
                                "baseUrl",
                                "getForObject",
                                "getForEntity",
                                "postForObject",
                                "postForEntity",
                                "exchange",
                            ]
                            .iter()
                            .any(|n| is(t, n))
                            {
                                continue;
                            }
                            if !stmt[..i]
                                .iter()
                                .any(|t| !t.string && clients.contains(&t.text))
                            {
                                continue;
                            }
                            let Some(args) = arguments(stmt, i) else {
                                continue;
                            };
                            let Some(first) = args.first() else {
                                continue;
                            };
                            let target =
                                if let Some(value) = literal_expression(args, &parameter_helpers) {
                                    host(&value)
                                } else if let Some(Some(value)) = literals.get(&first.text) {
                                    host(value)
                                } else if args.get(1).is_some_and(|t| is(t, "(")) {
                                    helpers.get(&first.text).map(|s| s.to_lowercase())
                                } else {
                                    None
                                };
                            if let Some(name) = target
                                && let Some(to) = unique(&targets, &name)
                            {
                                insert(
                                    &mut edges,
                                    from,
                                    &to,
                                    Kind::HttpCall,
                                    proof(
                                        input,
                                        "spring-client",
                                        Some(tokens[offset + i].line),
                                        "Spring HTTP client destination resolved from a literal, unique local string or literal DiscoveryClient helper or pure String parameter helper. Static source candidate; execution is not proven. URL values omitted.",
                                    ),
                                );
                            }
                        }
                        offset += stmt.len() + 1;
                    }
                }
                // Feign declarations are static clients, including service-discovery names.
                for (i, t) in tokens.iter().enumerate() {
                    if !(is(t, "@FeignClient")
                        || is(t, "FeignClient") && i > 0 && is(&tokens[i - 1], "@"))
                    {
                        continue;
                    }
                    let Some(args) = arguments(&tokens, i) else {
                        continue;
                    };
                    let name = args
                        .iter()
                        .enumerate()
                        .find(|(j, t)| {
                            t.string
                                && (*j == 0
                                    || *j >= 2
                                        && ["name", "value", "url"]
                                            .contains(&args[*j - 2].text.as_str()))
                        })
                        .map(|(_, t)| t.text.clone());
                    if let Some(name) = name {
                        let name = host(&name).unwrap_or(name);
                        if let Some(to) = unique(&targets, &name) {
                            insert(
                                &mut edges,
                                from,
                                &to,
                                Kind::HttpCall,
                                proof(
                                    input,
                                    "FeignClient",
                                    Some(t.line),
                                    "Literal Feign client declaration; runtime execution is not proven.",
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
    if publications.len() + subscriptions.len() >= 10_000 {
        diagnostic(
            report,
            "",
            None,
            "Spring messaging endpoint limit reached; remaining endpoints omitted.",
        );
    }
    let mut matched = 0;
    let mut listeners = BTreeMap::<_, Vec<_>>::new();
    for consumer in &subscriptions {
        listeners
            .entry((&consumer.protocol, &consumer.broker, &consumer.channel))
            .or_default()
            .push(consumer);
    }
    'messages: for producer in &publications {
        let Some(consumers) =
            listeners.get(&(&producer.protocol, &producer.broker, &producer.channel))
        else {
            continue;
        };
        for consumer in consumers {
            matched += 1;
            if matched > 100_000 {
                diagnostic(
                    report,
                    "",
                    None,
                    "Spring messaging match limit reached; remaining relationships omitted.",
                );
                break 'messages;
            }
            if service_scope(report, &producer.owner) == service_scope(report, &consumer.owner)
                && producer.broker == consumer.broker
                && producer.channel == consumer.channel
                && producer.protocol == consumer.protocol
            {
                insert(
                    &mut edges,
                    &producer.owner,
                    &consumer.owner,
                    Kind::Event,
                    producer.evidence.clone(),
                );
                insert(
                    &mut edges,
                    &producer.owner,
                    &consumer.owner,
                    Kind::Event,
                    consumer.evidence.clone(),
                );
            }
        }
    }
    for ((from, to, kind), evidence) in edges {
        report.dependencies.push(ServiceDependency {
            from,
            to,
            kind,
            evidence: evidence.into_iter().collect(),
        });
    }
    if !owners.is_empty() {
        diagnostic(
            report,
            "",
            None,
            "Spring coverage is partial: literal Gateway/Zuul routes, supported HTTP clients, configuration references and matching literal Kafka topics/Rabbit queues only. Profiles are combined; external configuration, dynamic endpoints and other messaging patterns are not evaluated.",
        );
    }
    if budget > 32_000_000 {
        diagnostic(report, "", None, "Spring source scan byte budget reached.");
    }
    report.services.sort_by(|a, b| a.id.cmp(&b.id));
}
