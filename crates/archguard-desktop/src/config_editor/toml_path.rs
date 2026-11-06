//! Concrete field paths (`layers[2].name`) and finding or creating their
//! place in a `toml_edit` document without touching anything else.

use toml_edit::{ArrayOfTables, InlineTable, Item, Table, TableLike, Value};

/// One step of a concrete path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// `architecture.model.components[1].include` → keys and indices.
pub fn parse(path: &str) -> Result<Vec<Seg>, String> {
    let mut segs = Vec::new();
    for part in path.split('.') {
        let (key, mut rest) = part.split_once('[').map_or((part, ""), |(k, r)| (k, r));
        if key.is_empty() {
            return Err(format!("{path}: empty key"));
        }
        segs.push(Seg::Key(key.to_string()));
        while !rest.is_empty() {
            let (index, after) = rest
                .split_once(']')
                .ok_or_else(|| format!("{path}: missing ]"))?;
            let index = index
                .parse()
                .map_err(|_| format!("{path}: {index:?} is not an entry index"))?;
            segs.push(Seg::Index(index));
            rest = after.strip_prefix('[').unwrap_or(after);
        }
    }
    Ok(segs)
}

/// The item at `segs`, if written.
pub fn get<'a>(root: &'a Table, segs: &[Seg]) -> Option<&'a Item> {
    let (last, parents) = segs.split_last()?;
    let mut table: &dyn TableLike = root;
    let mut iter = parents.iter().peekable();
    while let Some(seg) = iter.next() {
        let Seg::Key(key) = seg else { return None };
        let item = table.get(key)?;
        table = match iter.peek() {
            Some(Seg::Index(i)) => {
                iter.next();
                item.as_array_of_tables()?.get(*i)?
            }
            _ => item.as_table_like()?,
        };
    }
    match last {
        Seg::Key(key) => table.get(key),
        Seg::Index(_) => None,
    }
}

/// The table holding the last key of `segs`, created (as implicit
/// tables, which print no empty headers) where missing. Fails when a
/// written value or a missing array entry is in the way.
pub fn parent_mut<'a>(root: &'a mut Table, segs: &[Seg]) -> Result<&'a mut dyn TableLike, String> {
    let (_, parents) = segs.split_last().ok_or("empty path")?;
    let mut table: &mut dyn TableLike = root;
    let mut inline = false;
    let mut i = 0;
    while i < parents.len() {
        let Seg::Key(key) = &parents[i] else {
            return Err("index without a key".into());
        };
        let item = table.entry(key).or_insert_with(|| new_table(inline));
        inline = item.is_inline_table();
        table = match parents.get(i + 1) {
            Some(Seg::Index(n)) => {
                i += 1;
                let entries = item
                    .as_array_of_tables_mut()
                    .ok_or_else(|| format!("{key} is not a table array"))?;
                entries
                    .get_mut(*n)
                    .ok_or_else(|| format!("{key} has no entry {n}"))?
            }
            _ => item
                .as_table_like_mut()
                .ok_or_else(|| format!("{key} is not a table"))?,
        };
        i += 1;
    }
    Ok(table)
}

fn new_table(inline: bool) -> Item {
    if inline {
        Item::Value(Value::InlineTable(InlineTable::new()))
    } else {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    }
}

/// The last key of `segs`.
pub fn leaf(segs: &[Seg]) -> Result<&str, String> {
    match segs.last() {
        Some(Seg::Key(key)) => Ok(key),
        _ => Err("path must end with a key".into()),
    }
}

/// The `[[…]]` array at `segs`, created when missing.
pub fn array_mut<'a>(root: &'a mut Table, segs: &[Seg]) -> Result<&'a mut ArrayOfTables, String> {
    let key = leaf(segs)?.to_string();
    let parent = parent_mut(root, segs)?;
    let item = parent
        .entry(&key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    item.as_array_of_tables_mut()
        .ok_or_else(|| format!("{key} is not written as [[{key}]] entries; edit it as text"))
}
