use archguard_engine::{analyze_services, core::ServiceDependencyKind as Kind};
use std::{fs, path::Path};
fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}
fn project(root: &Path, path: &str, name: &str) {
    put(
        root,
        &format!("{path}/pom.xml"),
        &format!("<project><artifactId>{path}</artifactId></project>"),
    );
    put(
        root,
        &format!("{path}/src/main/resources/application.yml"),
        &format!("spring:\n  application:\n    name: {name}\n"),
    );
}
#[test]
fn spring_names_link_compose_routes_and_clients_to_projects() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "gateway", "gateway");
    project(r, "customers", "customers-service");
    project(r, "visits", "visits-service");
    put(
        r,
        "compose.yml",
        "services:\n  gateway:\n    image: demo/gateway\n  customers-service:\n    image: demo/customers\n  visits-service:\n    image: demo/visits\n",
    );
    put(
        r,
        "gateway/src/main/resources/application.yml",
        r#"spring:
  application:
    name: gateway
  cloud:
    gateway:
      server:
        webflux:
          routes:
            - id: customer
              uri: lb://customers-service
---
spring.config.activate.on-profile: docker
spring.cloud.gateway.routes:
  - id: visit
    uri: http://visits-service:8080/private?token=secret
"#,
    );
    put(
        r,
        "gateway/src/main/java/Clients.java",
        r#"
import org.springframework.web.reactive.function.client.WebClient;
class Clients {
 WebClient webClient;
 String hostname="http://visits-service/";
 void run(){webClient.get().uri(hostname+"pets").retrieve();}
 void other(){webClient.get().uri("http://customers-service/owners").retrieve();}
 // .uri("http://gateway/false")
 String documentation="http://gateway/false";
}
"#,
    );
    let result = analyze_services(r).unwrap();
    assert_eq!(
        result
            .services
            .iter()
            .filter(|s| s.project.is_some())
            .count(),
        3
    );
    assert_eq!(
        result
            .dependencies
            .iter()
            .filter(|e| e.kind == Kind::ConfiguredHttp)
            .count(),
        2
    );
    assert_eq!(
        result
            .dependencies
            .iter()
            .filter(|e| e.kind == Kind::HttpCall)
            .count(),
        2
    );
    assert!(!serde_json::to_string(&result).unwrap().contains("secret"));
    assert!(result.dependencies.iter().all(|e| !e.evidence.is_empty()));
}
#[test]
fn dynamic_config_and_unrelated_uris_are_not_calls() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "a", "a");
    project(r, "b", "b");
    put(
        r,
        "a/src/main/resources/application-prod.yml",
        "spring.cloud.gateway.routes:\n  - uri: ${TARGET:http://b}\n",
    );
    put(
        r,
        "a/src/main/java/Example.java",
        "class Example {void unused(){ arbitrary.uri(\"http://b\");} }",
    );
    assert!(analyze_services(r).unwrap().dependencies.is_empty());
}

#[test]
fn discovery_helper_and_feign_are_resolved_but_comments_are_not() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "a", "a");
    project(r, "b", "b");
    put(
        r,
        "a/src/main/java/Client.java",
        r#"
import org.springframework.web.client.RestClient;
class Client {
 RestClient restClient;
 void call(){return restClient.get().uri(getTarget()+"/owners").retrieve();}
 URI getTarget(){return discoveryClient.getInstances("b").get(0).getUri();}
}
"#,
    );
    put(
        r,
        "b/src/main/java/Client.java",
        r#"@FeignClient(name="a") interface Client {} // @FeignClient(name="b")"#,
    );
    let report = analyze_services(r).unwrap();
    assert_eq!(
        report
            .dependencies
            .iter()
            .filter(|e| e.kind == Kind::HttpCall)
            .count(),
        2
    );
    assert_eq!(
        report
            .cycles
            .iter()
            .filter(|c| c.kind == Kind::HttpCall)
            .count(),
        1
    );
}

#[test]
fn legacy_config_zuul_and_parameter_helper_have_source_evidence() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    for n in ["gateway", "catalog", "config"] {
        project(r, n, n);
    }
    put(
        r,
        "gateway/src/main/resources/bootstrap.yml",
        "spring.cloud.config.uri: http://config:8888\n",
    );
    put(
        r,
        "config/src/main/resources/application.yml",
        "spring.application.name: config\nspring.cloud.config.server.native.search-locations: classpath:/shared\n",
    );
    put(
        r,
        "config/src/main/resources/shared/gateway.yml",
        "zuul.routes.catalog.serviceId: catalog\n",
    );
    put(
        r,
        "gateway/src/main/java/Client.java",
        r#"
import org.springframework.web.client.RestTemplate;
class Client {
 RestTemplate restTemplate;
 private String endpoint(String serviceName) { return "http://" + serviceName; }
 void run() { String url = endpoint("catalog"); restTemplate.exchange(url + "/items", GET, null, String.class); }
}
"#,
    );
    let result = analyze_services(r).unwrap();
    for kind in [
        Kind::ConfiguredReference,
        Kind::ConfiguredHttp,
        Kind::HttpCall,
    ] {
        assert_eq!(
            result
                .dependencies
                .iter()
                .filter(|e| e.kind == kind)
                .count(),
            1,
            "{kind:?}"
        );
    }
    assert!(
        result
            .dependencies
            .iter()
            .find(|e| e.kind == Kind::ConfiguredHttp)
            .unwrap()
            .evidence
            .iter()
            .any(|e| e.file.ends_with("shared/gateway.yml"))
    );
}

#[test]
fn helper_dynamic_arguments_and_reassignments_stay_unknown() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "a", "a");
    project(r, "b", "b");
    put(
        r,
        "a/src/main/java/Client.java",
        r#"
import org.springframework.web.client.RestTemplate;
class Client {
 RestTemplate restTemplate;
 String endpoint(String name) { return "http://" + name; }
 void run(String name) { restTemplate.getForObject(endpoint(name), String.class); }
 void changed() { String url=endpoint("b"); url=System.getenv("URL"); restTemplate.getForObject(url,String.class); }
 String wrong(String name) { if (true) return "http://b"; return "http://" + name; }
 void notPure(){restTemplate.getForObject(wrong("b"),String.class);}
}
"#,
    );
    assert!(analyze_services(r).unwrap().dependencies.is_empty());
}

#[test]
fn kafka_topic_links_require_matching_literal_broker_and_typed_producer() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    for n in ["producer", "consumer", "other"] {
        project(r, n, n);
    }
    for n in ["producer", "consumer"] {
        put(
            r,
            &format!("{n}/src/main/resources/application.properties"),
            "spring.kafka.bootstrap-servers=broker:9092\n",
        );
    }
    put(
        r,
        "other/src/main/resources/application.properties",
        "spring.kafka.bootstrap-servers=other-broker:9092\n",
    );
    put(
        r,
        "producer/src/main/java/Producer.java",
        r#"
import org.springframework.kafka.core.KafkaTemplate;
class Producer { KafkaTemplate<String, String> kafka; void send(){ kafka.send("orders", "hello"); arbitrary.send("fake", "ignored"); } }
"#,
    );
    for n in ["consumer", "other"] {
        put(
            r,
            &format!("{n}/src/main/java/Consumer.java"),
            r#"
import org.springframework.kafka.annotation.KafkaListener;
class Consumer { @KafkaListener(topics="orders") void listen(String value){} }
"#,
        );
    }
    let report = analyze_services(r).unwrap();
    let events: Vec<_> = report
        .dependencies
        .iter()
        .filter(|e| e.kind == Kind::Event)
        .collect();
    assert_eq!(events.len(), 1);
    assert!(events[0].to.contains("consumer"));
    assert_eq!(events[0].evidence.len(), 2);
    assert!(
        events[0]
            .evidence
            .iter()
            .all(|e| e.description.contains("inferred"))
    );
}

#[test]
fn rabbit_default_exchange_matches_queues_but_named_exchange_does_not() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    for n in ["producer", "consumer", "unconfigured"] {
        project(r, n, n);
    }
    for n in ["producer", "consumer"] {
        put(
            r,
            &format!("{n}/src/main/resources/application.properties"),
            "spring.rabbitmq.host=broker\nspring.rabbitmq.virtual-host=/demo\n",
        );
    }
    put(
        r,
        "producer/src/main/java/Producer.java",
        r#"
import org.springframework.amqp.rabbit.core.RabbitTemplate;
class Producer { RabbitTemplate rabbit; void send(){ rabbit.convertAndSend("orders", "message"); rabbit.convertAndSend("fanout", "other", "message"); } }
"#,
    );
    for n in ["consumer", "unconfigured"] {
        put(
            r,
            &format!("{n}/src/main/java/Consumer.java"),
            r#"
import org.springframework.amqp.rabbit.annotation.RabbitListener;
class Consumer { @RabbitListener(queues="orders") void listen(String value){} @RabbitListener(queues="other") void other(String value){} }
"#,
        );
    }
    let result = analyze_services(r).unwrap();
    let events: Vec<_> = result
        .dependencies
        .iter()
        .filter(|e| e.kind == Kind::Event)
        .collect();
    assert_eq!(events.len(), 1);
    assert!(events[0].to.contains("consumer"));
    assert_eq!(events[0].evidence.len(), 2);
}

#[test]
fn unconfigured_shared_folder_is_not_a_config_repository() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "a", "a");
    project(r, "b", "b");
    put(
        r,
        "a/src/main/resources/shared/a.yml",
        "zuul.routes.b.serviceId: b\n",
    );
    assert!(analyze_services(r).unwrap().dependencies.is_empty());
}

#[test]
fn deployment_hostname_is_not_ambiguous_with_undeployed_manifest_label() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "client", "client");
    put(
        r,
        "config/pom.xml",
        "<project><artifactId>config</artifactId><packaging>war</packaging></project>",
    );
    put(
        r,
        "compose.yml",
        "services:\n  config:\n    image: example/config-server\n",
    );
    put(
        r,
        "client/src/main/resources/bootstrap.properties",
        "spring.cloud.config.uri=http://config:8888\n",
    );
    let result = analyze_services(r).unwrap();
    let edges: Vec<_> = result
        .dependencies
        .iter()
        .filter(|e| e.kind == Kind::ConfiguredReference)
        .collect();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].to, "compose.yml::config");
}

#[test]
fn same_service_names_in_two_compose_scopes_do_not_cross_environments() {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    project(r, "web", "web");
    project(r, "api", "api");
    for env in ["dev", "prod"] {
        put(
            r,
            &format!("{env}/compose.yaml"),
            "services:\n  web:\n    image: web\n  api:\n    image: api\n",
        );
    }
    put(
        r,
        "web/src/main/java/Client.java",
        r#"import org.springframework.web.client.RestTemplate; class C { RestTemplate client; void send(){ client.getForObject("http://api/data", String.class); } }"#,
    );
    let r = analyze_services(r).unwrap();
    let edges: Vec<_> = r
        .dependencies
        .iter()
        .filter(|e| e.kind == Kind::HttpCall)
        .collect();
    assert_eq!(edges.len(), 2);
    assert!(
        edges
            .iter()
            .all(|e| e.from.split('/').next() == e.to.split('/').next())
    );
}
