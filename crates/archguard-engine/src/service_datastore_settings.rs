//! Classification of datastore settings and connection URLs. Values are parsed
//! into engine, host, port and database name; credentials and options are dropped.
use archguard_core::ServiceEvidence;
use std::collections::BTreeMap;

/// A datastore setting with its value, kept in memory only, and the evidence of
/// where it was declared outside the workload (empty when declared in place).
pub(super) type Settings = BTreeMap<String, (String, Vec<ServiceEvidence>)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Url,
    Host,
    Port,
    Name,
}

const STORE_TOKENS: [&str; 17] = [
    "DB",
    "DATABASE",
    "DATASOURCE",
    "JDBC",
    "PG",
    "POSTGRES",
    "POSTGRESQL",
    "MYSQL",
    "MARIADB",
    "MONGO",
    "MONGODB",
    "REDIS",
    "CASSANDRA",
    "MSSQL",
    "SQLSERVER",
    "ORACLE",
    "ELASTICSEARCH",
];
const CREDENTIAL_TOKENS: [&str; 10] = [
    "PASSWORD",
    "PASS",
    "PWD",
    "SECRET",
    "TOKEN",
    "USER",
    "USERNAME",
    "KEY",
    "CREDENTIAL",
    "CREDENTIALS",
];

/// The role and grouping prefix of a datastore setting name, e.g. `DB_HOST` -> (Host, "DB").
/// Credential settings never have a role.
fn role(key: &str) -> Option<(Role, String)> {
    let key = key.to_ascii_uppercase();
    match key.as_str() {
        "PGHOST" => return Some((Role::Host, "PG".into())),
        "PGPORT" => return Some((Role::Port, "PG".into())),
        "PGDATABASE" => return Some((Role::Name, "PG".into())),
        _ => {}
    }
    let tokens: Vec<&str> = key.split('_').filter(|t| !t.is_empty()).collect();
    if tokens.len() < 2
        || tokens.iter().any(|t| CREDENTIAL_TOKENS.contains(t))
        || !tokens.iter().any(|t| STORE_TOKENS.contains(t))
    {
        return None;
    }
    let role = match *tokens.last()? {
        "HOST" | "HOSTNAME" | "SERVER" | "ADDR" | "ADDRESS" => Role::Host,
        "PORT" => Role::Port,
        "NAME" | "DB" | "DATABASE" | "SCHEMA" => Role::Name,
        "URL" | "URI" | "DSN" => Role::Url,
        _ => return None,
    };
    Some((role, tokens[..tokens.len() - 1].join("_")))
}

/// Whether a setting is kept for datastore resolution: a datastore setting name, or
/// any value that is a datastore connection URL.
pub(super) fn relevant(key: &str, value: &str) -> bool {
    value.len() <= 2048 && (role(key).is_some() || parse_url(value).is_some())
}

/// A datastore setting name, used to report values that come from Secrets.
pub(super) fn setting_name(key: &str) -> bool {
    role(key).is_some()
}

fn engine_of_prefix(prefix: &str) -> Option<&'static str> {
    prefix.split('_').find_map(|t| match t {
        "PG" | "POSTGRES" | "POSTGRESQL" => Some("postgresql"),
        "MYSQL" => Some("mysql"),
        "MARIADB" => Some("mariadb"),
        "MONGO" | "MONGODB" => Some("mongodb"),
        "REDIS" => Some("redis"),
        "CASSANDRA" => Some("cassandra"),
        "MSSQL" | "SQLSERVER" => Some("sqlserver"),
        "ORACLE" => Some("oracle"),
        "ELASTICSEARCH" => Some("elasticsearch"),
        _ => None,
    })
}

/// A parsed datastore reference. `host` is `None` when it could not be read literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reference {
    pub engine: Option<&'static str>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
}

fn hostname(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 253
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
    .then(|| value.to_ascii_lowercase())
}

fn database(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)))
    .then(|| value.to_owned())
}

/// `host[:port]` of an authority without user information.
fn host_port(authority: &str) -> (Option<String>, Option<u16>, bool) {
    if authority.contains([',', '[']) {
        return (None, None, false);
    }
    match authority.split_once(':') {
        Some((host, port)) => (
            hostname(host),
            port.parse().ok(),
            port.parse::<u16>().is_ok(),
        ),
        None => (hostname(authority), None, true),
    }
}

/// JDBC sub-protocols with a `//host` URL; Oracle has its own form.
const JDBC_URLS: [&str; 5] = ["postgresql", "postgres", "mysql", "mariadb", "sqlserver"];

/// The engine of a URL scheme or JDBC sub-protocol.
fn scheme_engine(scheme: &str) -> Option<&'static str> {
    Some(match scheme {
        "postgres" | "postgresql" => "postgresql",
        "mysql" => "mysql",
        "mariadb" => "mariadb",
        "mongodb" | "mongodb+srv" => "mongodb",
        "redis" | "rediss" => "redis",
        "sqlserver" | "mssql" => "sqlserver",
        _ => return None,
    })
}

/// Engine, host, port and database of a connection URL; `None` if it is no datastore URL.
pub(super) fn parse_url(raw: &str) -> Option<Reference> {
    let value = raw.trim();
    let lower = value.to_ascii_lowercase();
    let (engine, rest) = if let Some(tail) = lower.strip_prefix("jdbc:") {
        let (sub, _) = tail.split_once(':')?;
        let offset = "jdbc:".len() + sub.len() + 1;
        if sub == "oracle" {
            return Some(oracle(&value[offset..]));
        }
        let engine = scheme_engine(sub).filter(|_| JDBC_URLS.contains(&sub))?;
        (engine, value[offset..].strip_prefix("//")?)
    } else {
        let start = if lower.starts_with("r2dbc:") { 6 } else { 0 };
        let (scheme, _) = lower[start..].split_once("://")?;
        (scheme_engine(scheme)?, &value[start + scheme.len() + 3..])
    };
    // Only the part after the last `@` names the server; user information is dropped.
    let before_query = rest.split(['?', '#']).next().unwrap_or("");
    let server = before_query
        .rfind('@')
        .map_or(before_query, |at| &before_query[at + 1..]);
    let end = server.find(['/', ';']).unwrap_or(server.len());
    let (host, port, valid) = host_port(&server[..end]);
    let tail = &server[end..];
    let name = if engine == "sqlserver" {
        tail.split(';')
            .filter_map(|part| part.split_once('='))
            .find(|(k, _)| matches!(k.to_ascii_lowercase().as_str(), "databasename" | "database"))
            .and_then(|(_, v)| database(v))
    } else {
        tail.strip_prefix('/')
            .and_then(|p| p.split(['/', ';']).next())
            .and_then(database)
    };
    Some(Reference {
        engine: Some(engine),
        host: host.filter(|_| valid),
        port,
        database: name,
    })
}

/// `thin:@//host:port/service`, `thin:@host:port:SID` or `thin:user/pass@host...`.
fn oracle(rest: &str) -> Reference {
    let server = rest.rsplit('@').next().unwrap_or("");
    let server = server.strip_prefix("//").unwrap_or(server);
    let mut parts = server.splitn(2, [':', '/']);
    let host = parts.next().and_then(hostname);
    let rest = parts.next().unwrap_or("");
    let (port, name) = match rest.split_once(['/', ':']) {
        Some((port, name)) => (port.parse().ok(), database(name)),
        None => (rest.parse().ok(), None),
    };
    Reference {
        engine: Some("oracle"),
        host: if server.contains(['(', ',']) {
            None
        } else {
            host
        },
        port,
        database: name,
    }
}

/// References of one workload: URLs, then host settings grouped with name and port
/// settings of the same prefix (or the only ones, when a single host is configured).
pub(super) fn references(settings: &Settings) -> Vec<(Reference, Vec<String>)> {
    let mut out = Vec::new();
    let mut roles: Vec<(Role, String, &str, &str)> = Vec::new();
    for (key, (value, _)) in settings {
        if let Some(reference) = parse_url(value) {
            out.push((reference, vec![key.clone()]));
        } else if let Some((role, prefix)) = role(key)
            && role != Role::Url
        {
            roles.push((role, prefix, key, value));
        }
    }
    let hosts = roles.iter().filter(|r| r.0 == Role::Host).count();
    // The setting of `role` with the host's prefix, or the only one when there is one host.
    let pick = |role: Role, prefix: &str| -> Option<(String, String)> {
        let list: Vec<_> = roles.iter().filter(|r| r.0 == role).collect();
        list.iter()
            .find(|r| r.1 == prefix)
            .or_else(|| (hosts == 1 && list.len() == 1).then(|| &list[0]))
            .map(|r| (r.2.to_owned(), r.3.to_owned()))
    };
    for (_, prefix, key, value) in roles.iter().filter(|r| r.0 == Role::Host) {
        let (host, inline_port, valid) = host_port(value);
        let mut keys = vec![(*key).to_owned()];
        let name = pick(Role::Name, prefix).and_then(|(k, v)| {
            keys.push(k);
            database(&v)
        });
        let port = inline_port.or_else(|| {
            pick(Role::Port, prefix).and_then(|(k, v)| {
                keys.push(k);
                v.parse().ok()
            })
        });
        out.push((
            Reference {
                engine: engine_of_prefix(prefix),
                host: host.filter(|_| valid),
                port,
                database: name,
            },
            keys,
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_keep_only_engine_host_port_and_database() {
        let parsed = |raw| parse_url(raw).unwrap();
        assert_eq!(
            parsed("postgres://u:p@ss/w@db:5432/shop?password=x"),
            Reference {
                engine: Some("postgresql"),
                host: Some("db".into()),
                port: Some(5432),
                database: Some("shop".into())
            }
        );
        let server = parsed("jdbc:sqlserver://sql:1433;databaseName=Sales;password=x");
        assert_eq!(server.host.as_deref(), Some("sql"));
        assert_eq!(server.database.as_deref(), Some("Sales"));
        let oracle = parsed("jdbc:oracle:thin:scott/tiger@//ora:1521/ORCL");
        assert_eq!(oracle.host.as_deref(), Some("ora"));
        assert_eq!(oracle.database.as_deref(), Some("ORCL"));
        assert_eq!(parsed("mongodb://a,b/x").host, None);
        assert_eq!(
            parsed("redis://cache:6379/0").database.as_deref(),
            Some("0")
        );
        assert_eq!(
            parsed("jdbc:postgresql://${DB_HOST}:5432/app").host,
            None,
            "interpolated hosts stay unknown"
        );
        assert!(parse_url("http://db:5432/app").is_none());
        assert!(parse_url("jdbc:h2:mem:test").is_none());
    }

    #[test]
    fn credential_settings_have_no_role() {
        assert!(role("DB_PASSWORD").is_none());
        assert!(role("POSTGRES_USER").is_none());
        assert!(role("REDIS_KEY_PREFIX").is_none());
        assert!(role("DB_HOST").is_some_and(|(r, p)| r == Role::Host && p == "DB"));
        assert!(role("POSTGRES_DB").is_some_and(|(r, p)| r == Role::Name && p == "POSTGRES"));
        assert!(role("SERVICE_HOST").is_none());
    }
}
