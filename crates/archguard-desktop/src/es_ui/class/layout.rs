#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! Column layout and edge paths, ticket 08.

use super::boxes::BoxContent;
use super::diagram::{Diagram, EdgeStyle};
use crate::diagram::HeadShape;

/// A positioned box. `comps` are the compartment tops (relative to the box); `chips` are
/// `(x, y, w)` of all chips in compartment order, relative to their compartment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placed {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub comps: Vec<f32>,
    pub chips: Vec<(f32, f32, f32)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedEdge {
    pub line: String,
    pub head: String,
    pub shape: HeadShape,
    pub dashed: bool,
    pub style: EdgeStyle,
    pub label: String,
    pub lx: f32,
    pub ly: f32,
}

/// `boxes` is index-aligned with `Diagram::boxes`, `edges` follows `Diagram::edges`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub boxes: Vec<Placed>,
    pub edges: Vec<PlacedEdge>,
    pub width: f32,
    pub height: f32,
}

pub fn layout(_d: &Diagram, _contents: &[BoxContent]) -> Scene {
    Scene::default()
}
