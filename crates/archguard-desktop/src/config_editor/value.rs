//! A field's value as a form edits it, and its conversion to and from TOML.

use std::fmt;

use toml_edit::{Array, InlineTable, Item, TableLike, Value};

use super::catalogue::Kind;

/// A typed field value. Enum fields hold [`FieldValue::Text`].
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<String>),
    /// Name → number, in file order.
    Map(Vec<(String, f64)>),
}

impl FieldValue {
    /// Read `item` as a value of `kind`; `None` when the TOML has another
    /// type (validation then names the field).
    pub fn from_item(kind: Kind, item: &Item) -> Option<FieldValue> {
        if kind == Kind::Map {
            return numbers(item.as_table_like()?).map(FieldValue::Map);
        }
        let value = item.as_value()?;
        match kind {
            Kind::Bool => value.as_bool().map(FieldValue::Bool),
            Kind::Int => value.as_integer().map(FieldValue::Int),
            Kind::Float => value
                .as_float()
                .or_else(|| value.as_integer().map(|i| i as f64))
                .map(FieldValue::Float),
            Kind::Text | Kind::Enum(_) => value.as_str().map(|s| FieldValue::Text(s.into())),
            Kind::List => strings(value.as_array()?).map(FieldValue::List),
            Kind::Map | Kind::Table | Kind::TableArray => None,
        }
    }

    /// The TOML value, formatted the way `toml_edit` writes new values.
    pub fn to_value(&self) -> Value {
        match self {
            FieldValue::Bool(b) => Value::from(*b),
            FieldValue::Int(i) => Value::from(*i),
            FieldValue::Float(x) => Value::from(*x),
            FieldValue::Text(s) => Value::from(s.as_str()),
            FieldValue::List(items) => Value::Array(items.iter().map(String::as_str).collect()),
            FieldValue::Map(pairs) => {
                let mut table = InlineTable::new();
                for (name, weight) in pairs {
                    table.insert(name, Value::from(*weight));
                }
                Value::InlineTable(table)
            }
        }
    }

    /// Whether this value can be stored in a field of `kind`.
    pub fn fits(&self, kind: Kind) -> bool {
        match (self, kind) {
            (FieldValue::Text(s), Kind::Enum(allowed)) => allowed.contains(&s.as_str()),
            (FieldValue::Bool(_), Kind::Bool)
            | (FieldValue::Int(_), Kind::Int)
            | (FieldValue::Float(_), Kind::Float)
            | (FieldValue::Text(_), Kind::Text)
            | (FieldValue::List(_), Kind::List)
            | (FieldValue::Map(_), Kind::Map) => true,
            _ => false,
        }
    }

    /// Parse what a user typed into a text input for a field of `kind`:
    /// lists one entry per line, maps one `name = number` per line (the
    /// inverse of [`fmt::Display`]).
    pub fn parse(kind: Kind, input: &str) -> Result<FieldValue, String> {
        let input = input.trim();
        let value = match kind {
            Kind::Bool => match input {
                "true" => FieldValue::Bool(true),
                "false" => FieldValue::Bool(false),
                _ => return Err("expected true or false".into()),
            },
            Kind::Int => FieldValue::Int(
                input
                    .parse::<u32>()
                    .map_err(|_| "expected a whole number of at least 0".to_string())?
                    .into(),
            ),
            Kind::Float => FieldValue::Float(
                input
                    .parse::<f64>()
                    .ok()
                    .filter(|x| x.is_finite())
                    .ok_or("expected a number")?,
            ),
            Kind::Text | Kind::Enum(_) => FieldValue::Text(input.into()),
            Kind::List => FieldValue::List(lines(input).map(String::from).collect()),
            Kind::Map => FieldValue::Map(lines(input).map(map_line).collect::<Result<_, _>>()?),
            Kind::Table | Kind::TableArray => return Err("a section has no value".into()),
        };
        if value.fits(kind) {
            Ok(value)
        } else {
            Err(match kind {
                Kind::Enum(allowed) => format!("expected one of {}", allowed.join(", ")),
                _ => "value does not fit the field".into(),
            })
        }
    }
}

impl fmt::Display for FieldValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldValue::Bool(b) => write!(f, "{b}"),
            FieldValue::Int(i) => write!(f, "{i}"),
            FieldValue::Float(x) => write!(f, "{x}"),
            FieldValue::Text(s) => f.write_str(s),
            FieldValue::List(items) => f.write_str(&items.join("\n")),
            FieldValue::Map(pairs) => {
                let lines: Vec<String> = pairs.iter().map(|(k, v)| format!("{k} = {v}")).collect();
                f.write_str(&lines.join("\n"))
            }
        }
    }
}

fn strings(array: &Array) -> Option<Vec<String>> {
    array.iter().map(|v| v.as_str().map(String::from)).collect()
}

fn numbers(table: &dyn TableLike) -> Option<Vec<(String, f64)>> {
    table
        .iter()
        .map(|(k, item)| {
            let v = item.as_value()?;
            let x = v.as_float().or_else(|| v.as_integer().map(|i| i as f64))?;
            Some((k.to_string(), x))
        })
        .collect()
}

fn lines(input: &str) -> impl Iterator<Item = &str> {
    input.lines().map(str::trim).filter(|l| !l.is_empty())
}

fn map_line(line: &str) -> Result<(String, f64), String> {
    let (name, number) = line
        .split_once('=')
        .ok_or_else(|| format!("{line:?}: expected name = number"))?;
    let number = number
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{line:?}: expected a number after ="))?;
    Ok((name.trim().to_string(), number))
}
