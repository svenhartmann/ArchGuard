//! The entity diagram: one band per aggregate (its root, then the
//! entities and value objects it holds, column by distance), entities of
//! no aggregate in a last band; relations as arrows with multiplicity,
//! `references` dashed.

use std::collections::HashMap;

use archguard_engine::core::domain_model::{DomainModel, EntityKind, RelationKind};

use crate::diagram;

pub const CARD_W: f32 = 190.0;
pub const CARD_H: f32 = 52.0;
const GAP_X: f32 = 70.0;
const GAP_Y: f32 = 16.0;
const HEAD: f32 = 30.0;
const PAD: f32 = 20.0;
const BAND_GAP: f32 = 18.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// Index into the model's entities.
    pub entity: usize,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub path: String,
    pub dashed: bool,
    pub label: &'static str,
    pub at: diagram::Point,
}

/// A band: the root's entity index, or `None` for entities of no aggregate.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub root: Option<usize>,
    pub y: f32,
    pub h: f32,
}

#[derive(Debug, Default)]
pub struct Diagram {
    pub cards: Vec<Card>,
    pub arrows: Vec<Arrow>,
    pub bands: Vec<Band>,
    pub width: f32,
    pub height: f32,
}

/// Each entity's band (root index or `None`) and column, by breadth first
/// search from every aggregate root along `has_one`/`has_many`; another
/// root ends the search.
pub fn membership(model: &DomainModel) -> Vec<(Option<usize>, usize)> {
    let ids: HashMap<&str, usize> = (model.entities.iter().enumerate())
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let root = |i: usize| model.entities[i].kind == EntityKind::AggregateRoot;
    let mut out: Vec<Option<(Option<usize>, usize)>> = vec![None; model.entities.len()];
    let mut roots: Vec<usize> = (0..model.entities.len()).filter(|&i| root(i)).collect();
    roots.sort_by(|&a, &b| model.entities[a].name.cmp(&model.entities[b].name));
    for r in roots {
        out[r] = Some((Some(r), 0));
        let mut queue = vec![r];
        while let Some(i) = queue.pop() {
            let col = out[i].map_or(0, |(_, c)| c);
            let owned = (model.relations.iter())
                .filter(|x| x.from == model.entities[i].id && x.kind != RelationKind::References);
            for to in owned.filter_map(|x| ids.get(x.to.as_str()).copied()) {
                if out[to].is_none() && !root(to) {
                    out[to] = Some((Some(r), col + 1));
                    queue.insert(0, to);
                }
            }
        }
    }
    out.into_iter().map(|m| m.unwrap_or((None, 0))).collect()
}

pub fn layout(model: &DomainModel) -> Diagram {
    let member = membership(model);
    let mut bands: Vec<Option<usize>> = Vec::new();
    for (band, _) in &member {
        if band.is_some() && !bands.contains(band) {
            bands.push(*band);
        }
    }
    bands.sort_by_key(|b| b.map(|r| &model.entities[r].name));
    if member.iter().any(|(b, _)| b.is_none()) {
        bands.push(None);
    }
    let mut d = Diagram::default();
    let mut place: Vec<(f32, f32)> = vec![(0.0, 0.0); model.entities.len()];
    let mut top = PAD;
    for band in bands {
        let mut rows: HashMap<usize, usize> = HashMap::new();
        let mut order: Vec<usize> = (0..member.len()).filter(|&i| member[i].0 == band).collect();
        order.sort_by(|&a, &b| {
            (member[a].1, &model.entities[a].name).cmp(&(member[b].1, &model.entities[b].name))
        });
        for i in order {
            // Entities of no aggregate fill a row of up to four columns.
            let (col, row) = match band {
                Some(_) => {
                    let r = rows.entry(member[i].1).or_insert(0);
                    *r += 1;
                    (member[i].1, *r - 1)
                }
                None => {
                    let n = rows.entry(0).or_insert(0);
                    *n += 1;
                    ((*n - 1) % 4, (*n - 1) / 4)
                }
            };
            let (x, y) = (
                PAD + col as f32 * (CARD_W + GAP_X),
                top + HEAD + row as f32 * (CARD_H + GAP_Y),
            );
            place[i] = (x, y);
            d.cards.push(Card { entity: i, x, y });
            d.width = d.width.max(x + CARD_W + PAD);
        }
        let most = match band {
            Some(_) => rows.values().copied().max().unwrap_or(0),
            None => rows.get(&0).map_or(0, |n| n.div_ceil(4)),
        };
        let h = HEAD + most as f32 * (CARD_H + GAP_Y) + 4.0;
        d.bands.push(Band {
            root: band,
            y: top,
            h,
        });
        top += h + BAND_GAP;
    }
    d.height = top;
    d.arrows = arrows(model, &place);
    d
}

/// Multiplicity of a relation as shown on its arrow.
pub fn multiplicity(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::HasOne => "1",
        RelationKind::HasMany => "*",
        RelationKind::References => "id",
    }
}

fn arrows(model: &DomainModel, place: &[(f32, f32)]) -> Vec<Arrow> {
    let ids: HashMap<&str, usize> = (model.entities.iter().enumerate())
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let mut seen: HashMap<(usize, usize), f32> = HashMap::new();
    let mut out = Vec::new();
    for r in &model.relations {
        let (Some(&a), Some(&b)) = (ids.get(r.from.as_str()), ids.get(r.to.as_str())) else {
            continue;
        };
        // A second relation between the same pair runs beside the first.
        let shift = seen.entry((a.min(b), a.max(b))).or_insert(-18.0);
        *shift += 18.0;
        let (p, q) = ends(place[a], place[b], *shift);
        let dashed = r.kind == RelationKind::References;
        let (path, at) = diagram::line(p, q, dashed);
        out.push(Arrow {
            path,
            dashed,
            label: multiplicity(r.kind),
            at,
        });
    }
    out
}

/// The facing sides of two cards: left/right in different columns, else
/// bottom/top; `shift` moves both ends sideways.
fn ends(a: (f32, f32), b: (f32, f32), shift: f32) -> (diagram::Point, diagram::Point) {
    if (a.0 - b.0).abs() > 1.0 {
        let (p, q) = diagram::ends(a, b, CARD_W, CARD_H);
        ((p.0, p.1 + shift), (q.0, q.1 + shift))
    } else if b.1 > a.1 {
        (
            (a.0 + CARD_W / 2.0 + shift, a.1 + CARD_H),
            (b.0 + CARD_W / 2.0 + shift, b.1),
        )
    } else {
        (
            (a.0 + CARD_W / 2.0 + shift, a.1),
            (b.0 + CARD_W / 2.0 + shift, b.1 + CARD_H),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{Entity, Relation};

    fn entity(name: &str, kind: EntityKind) -> Entity {
        Entity {
            id: name.into(),
            name: name.into(),
            kind,
            fields: vec![],
            file: format!("{name}.cs"),
            line: 1,
            source: "t".into(),
            module: String::new(),
        }
    }

    fn rel(from: &str, to: &str, kind: RelationKind) -> Relation {
        Relation {
            from: from.into(),
            to: to.into(),
            kind,
            via: to.into(),
            source: "t".into(),
            external: false,
        }
    }

    pub fn shop() -> DomainModel {
        use EntityKind::*;
        use RelationKind::*;
        DomainModel {
            entities: vec![
                entity("Order", AggregateRoot),
                entity("OrderItem", Entity),
                entity("Address", ValueObject),
                entity("Buyer", AggregateRoot),
                entity("CatalogItem", Unknown),
            ],
            relations: vec![
                rel("Order", "OrderItem", HasMany),
                rel("Order", "Address", HasOne),
                rel("Order", "Buyer", HasOne),
                rel("Order", "Buyer", References),
            ],
            ..DomainModel::default()
        }
    }

    #[test]
    fn aggregates_form_bands_with_the_root_first() {
        let d = layout(&shop());
        let roots: Vec<Option<usize>> = d.bands.iter().map(|b| b.root).collect();
        assert_eq!(
            roots,
            [Some(3), Some(0), None],
            "Buyer, Order, then the rest"
        );
        let at = |i| {
            d.cards
                .iter()
                .find(|c| c.entity == i)
                .map(|c| (c.x, c.y))
                .unwrap()
        };
        let order_top = d.bands[1].y + HEAD;
        assert_eq!(at(0), (PAD, order_top));
        assert_eq!(
            at(2),
            (PAD + CARD_W + GAP_X, order_top),
            "Address beside the root"
        );
        assert_eq!(at(1).0, at(2).0, "OrderItem below Address");
        assert!(at(1).1 > at(2).1);
        assert_eq!(at(4).1, d.bands[2].y + HEAD, "no aggregate: last band");
    }

    #[test]
    fn relations_carry_multiplicity_and_references_are_dashed() {
        let d = layout(&shop());
        let labels: Vec<(&str, bool)> = d.arrows.iter().map(|a| (a.label, a.dashed)).collect();
        assert_eq!(
            labels,
            [("*", false), ("1", false), ("1", false), ("id", true)]
        );
        assert_ne!(
            d.arrows[2].at, d.arrows[3].at,
            "two relations of one pair apart"
        );
    }
}
