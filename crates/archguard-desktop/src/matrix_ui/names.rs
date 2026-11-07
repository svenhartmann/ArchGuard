//! Labels of the Matrix's top rows and columns: the building-block names
//! of [`crate::block_names`], as in the arc42 views.

use std::collections::BTreeMap;

use crate::block_names::short_names;
use crate::tree::Tree;

/// Label per node of `roots`, from their full names.
pub fn root_labels(tree: &Tree, roots: &[usize]) -> BTreeMap<usize, String> {
    let names: Vec<&str> = roots.iter().map(|&r| tree.nodes[r].name.as_str()).collect();
    roots.iter().copied().zip(short_names(&names)).collect()
}
