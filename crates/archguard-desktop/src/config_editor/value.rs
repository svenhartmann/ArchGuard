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
            Kind::Float => number(value).map(FieldValue::Float),
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

    /// Write this value over `item`, which holds the old one. A list keeps
    /// the items that stay with their comments; a table (standard or
    /// inline) keeps the entries that stay as written. Any other value is
    /// replaced, keeping the comments around it.
    pub fn update(&self, item: &mut Item) {
        if let (FieldValue::List(items), Some(array)) = (self, item.as_array_mut()) {
            return update_list(array, items);
        }
        if let (FieldValue::Map(pairs), Some(table)) = (self, item.as_table_like_mut()) {
            return update_map(table, pairs);
        }
        let mut new = self.to_value();
        if let Some(old) = item.as_value() {
            *new.decor_mut() = old.decor().clone();
        }
        *item = Item::Value(new);
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
        .map(|(k, item)| Some((k.to_string(), number(item.as_value()?)?)))
        .collect()
}

fn number(value: &Value) -> Option<f64> {
    value
        .as_float()
        .or_else(|| value.as_integer().map(|i| i as f64))
}

fn prefix(value: &Value) -> Option<&str> {
    value.decor().prefix()?.as_str()
}

/// Rebuild `array` as `items`: an item that stays keeps its comments, a
/// new one goes on a line of its own in a multi-line array.
fn update_list(array: &mut Array, items: &[String]) {
    // Without a trailing comma the text before `]` is the last value's
    // suffix; move it to `trailing`, which stays at the end.
    if !array.trailing_comma()
        && let Some(last) = array.iter_mut().last()
    {
        let suffix = last.decor().suffix().and_then(|s| s.as_str()).unwrap_or("");
        let suffix = suffix.to_string();
        last.decor_mut().set_suffix("");
        let trailing = array.trailing().as_str().unwrap_or("");
        array.set_trailing(format!("{suffix}{trailing}"));
    }
    let old: Vec<Value> = array.iter().cloned().collect();
    let mut used = vec![false; old.len()];
    let picks: Vec<Option<usize>> = items
        .iter()
        .map(|item| {
            let i = (0..old.len()).find(|&i| !used[i] && old[i].as_str() == Some(item))?;
            used[i] = true;
            Some(i)
        })
        .collect();
    // A comment after a value's comma opens the next prefix (or `trailing`);
    // split it off so it stays with its value. `lines[i + 1]` follows
    // value `i`, `lines[0]` follows `[`.
    let (mut lines, owns): (Vec<&str>, Vec<Option<&str>>) = old
        .iter()
        .map(|v| match prefix(v) {
            Some(p) => {
                let (line, own) = split_line(p);
                (line, Some(own))
            }
            None => ("", None),
        })
        .unzip();
    let trailing = array.trailing().as_str().unwrap_or("").to_string();
    let (line, rest) = split_line(&trailing);
    lines.push(line);
    let indent = owns
        .last()
        .copied()
        .flatten()
        .and_then(|p| p.rsplit_once('\n'))
        .map(|(_, indent)| format!("\n{indent}"));
    array.clear();
    let mut carry = lines[0];
    for (pick, item) in picks.iter().zip(items) {
        let (mut value, own) = match pick {
            Some(i) => (old[*i].clone(), owns[*i]),
            None => (Value::from(item.as_str()), indent.as_deref()),
        };
        if let Some(prefix) = join(carry, own) {
            value.decor_mut().set_prefix(prefix);
        }
        carry = pick.map_or("", |i| lines[i + 1]);
        array.push_formatted(value);
    }
    array.set_trailing(join(carry, Some(rest)).unwrap_or_default());
}

/// The text before a value (or `]`): the comment ending the previous
/// line, and the rest from the first line break on.
fn split_line(text: &str) -> (&str, &str) {
    text.find('\n').map_or(("", text), |i| text.split_at(i))
}

/// A same-line comment, then `own` on the next line; `None`: default
/// formatting.
fn join(line: &str, own: Option<&str>) -> Option<String> {
    match own {
        _ if line.is_empty() => own.map(String::from),
        Some(own) if own.starts_with('\n') => Some(format!("{line}{own}")),
        own => Some(format!("{line}\n{}", own.unwrap_or(""))),
    }
}

/// Set `table` to `pairs`: entries that stay keep their formatting, a
/// changed weight keeps its comments and stays an integer if it was one
/// and still is whole.
fn update_map(table: &mut dyn TableLike, pairs: &[(String, f64)]) {
    let dropped: Vec<String> = table
        .iter()
        .map(|(k, _)| k.to_string())
        .filter(|k| !pairs.iter().any(|(name, _)| name == k))
        .collect();
    for key in dropped {
        table.remove(&key);
    }
    for (name, weight) in pairs {
        let Some(old) = table.get_mut(name).and_then(Item::as_value_mut) else {
            table.insert(name, Item::Value(Value::from(*weight)));
            continue;
        };
        if number(old) == Some(*weight) {
            continue;
        }
        let mut new = if old.is_integer() && weight.fract() == 0.0 {
            Value::from(*weight as i64)
        } else {
            Value::from(*weight)
        };
        *new.decor_mut() = old.decor().clone();
        *old = new;
    }
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
