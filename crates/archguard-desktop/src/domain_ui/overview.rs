//! The entity diagram of a large model: an overview with one box per module
//! and bundled arrows, and the band diagram of one module with a stub per
//! foreign module. Entities that many modules point to (hubs) are listed as
//! badges instead of drawn as arrows. Small models keep the plain band
//! diagram. Modules come from `Entity::module`, never from names.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use archguard_engine::core::domain_model::{DomainModel, EntityKind};

use super::entities::{self, Diagram};
use crate::diagram;

/// Above this many entities the diagram opens as the overview, aggregates or not.
pub const OVERVIEW_MIN_ENTITIES: usize = 40;
/// An entity targeted by relations from at least this many other modules is a hub.
pub const HUB_MIN_MODULES: usize = 3;

const BOX_W: f32 = 260.0;
const BOX_H: f32 = 64.0;
const STUB_W: f32 = 170.0;
const STUB_H: f32 = 34.0;
const GAP_X: f32 = 80.0;
const GAP_Y: f32 = 70.0;
const PAD: f32 = 20.0;

/// A module box of the overview, or a stub of the module view.
#[derive(Debug, Clone, PartialEq)]
pub struct ModBox {
    /// Index into [`modules`].
    pub module: usize,
    pub title: String,
    pub sub: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The stub of relations to models outside the sources (`Relation::external`).
    pub external: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub path: String,
    pub count: usize,
    pub at: diagram::Point,
}

/// An entity targeted from `from` relations of other modules.
#[derive(Debug, Clone, PartialEq)]
pub struct Hub {
    pub entity: usize,
    pub from: usize,
}

#[derive(Debug, Default)]
pub struct Scene {
    pub diagram: Diagram,
    pub boxes: Vec<ModBox>,
    pub links: Vec<Link>,
    pub hubs: Vec<Hub>,
}

/// The distinct modules, the largest first, then by name.
pub fn modules(model: &DomainModel) -> Vec<String> {
    let mut n: BTreeMap<&str, usize> = BTreeMap::new();
    for e in &model.entities {
        *n.entry(e.module.as_str()).or_default() += 1;
    }
    let mut out: Vec<(&str, usize)> = n.into_iter().collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    out.into_iter().map(|(m, _)| m.to_string()).collect()
}

/// Whether the diagram opens as the overview: several modules, and no
/// aggregates or more than [`OVERVIEW_MIN_ENTITIES`] entities.
pub fn wanted(model: &DomainModel) -> bool {
    let roots = model
        .entities
        .iter()
        .any(|e| e.kind == EntityKind::AggregateRoot);
    modules(model).len() > 1 && (!roots || model.entities.len() > OVERVIEW_MIN_ENTITIES)
}

/// Relations between known entities as index pairs.
fn pairs(model: &DomainModel) -> Vec<(usize, usize)> {
    let ids: HashMap<&str, usize> = (model.entities.iter().enumerate())
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let at = |id: &str| ids.get(id).copied();
    (model.relations.iter())
        .filter_map(|r| Some((at(&r.from)?, at(&r.to)?)))
        .collect()
}

/// Hubs, the most pointed-to first.
fn hubs(model: &DomainModel, pairs: &[(usize, usize)]) -> Vec<Hub> {
    let module = |i: usize| model.entities[i].module.as_str();
    let mut by: BTreeMap<usize, (BTreeSet<&str>, usize)> = BTreeMap::new();
    for &(a, b) in pairs.iter().filter(|(a, b)| module(*a) != module(*b)) {
        let t = by.entry(b).or_default();
        t.0.insert(module(a));
        t.1 += 1;
    }
    let mut out: Vec<Hub> = (by.into_iter())
        .filter(|(_, (m, _))| m.len() >= HUB_MIN_MODULES)
        .map(|(entity, (_, from))| Hub { entity, from })
        .collect();
    out.sort_by(|a, b| b.from.cmp(&a.from).then(a.entity.cmp(&b.entity)));
    out
}

/// What the entity diagram shows: the overview when `module` is `None` and
/// the model [`wanted`] it, one module's bands, or the plain band diagram.
pub fn scene(model: &DomainModel, module: Option<&str>) -> Scene {
    let pairs = pairs(model);
    let all = hubs(model, &pairs);
    match module {
        None if wanted(model) => overview(model, &pairs, all),
        None => Scene {
            diagram: entities::layout(model),
            ..Scene::default()
        },
        Some(m) => of_module(model, m, &pairs, all),
    }
}

fn overview(model: &DomainModel, pairs: &[(usize, usize)], hubs: Vec<Hub>) -> Scene {
    let names = modules(model);
    let at = |i: usize| names.iter().position(|n| *n == model.entities[i].module);
    let hub = |i: usize| hubs.iter().any(|h| h.entity == i);
    let mut degree = vec![0usize; model.entities.len()];
    let mut bundled: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for &(a, b) in pairs {
        degree[a] += 1;
        degree[b] += 1;
        if let (Some(x), Some(y)) = (at(a), at(b))
            && x != y
            && !hub(b)
        {
            *bundled.entry((x.min(y), x.max(y))).or_default() += 1;
        }
    }
    let cols = (names.len() as f32).sqrt().ceil() as usize;
    let place = |k: usize| {
        (
            PAD + (k % cols) as f32 * (BOX_W + GAP_X),
            PAD + (k / cols) as f32 * (BOX_H + GAP_Y),
        )
    };
    let boxes: Vec<ModBox> = (names.iter().enumerate())
        .map(|(k, name)| {
            let mut own: Vec<usize> = (0..model.entities.len())
                .filter(|&i| model.entities[i].module == *name)
                .collect();
            let total = own.len();
            own.sort_by(|&a, &b| {
                (degree[b].cmp(&degree[a]))
                    .then(model.entities[a].name.cmp(&model.entities[b].name))
            });
            let top: Vec<&str> = own
                .iter()
                .take(3)
                .map(|&i| model.entities[i].name.as_str())
                .collect();
            let (x, y) = place(k);
            ModBox {
                module: k,
                title: format!("{name} ({total})"),
                sub: top.join(", "),
                x,
                y,
                w: BOX_W,
                h: BOX_H,
                external: false,
            }
        })
        .collect();
    let links = (bundled.into_iter())
        .map(|((a, b), count)| {
            let (p, q) = diagram::ends(place(a), place(b), BOX_W, BOX_H);
            let (path, at) = diagram::line(p, q, false);
            Link { path, count, at }
        })
        .collect();
    let rows = names.len().div_ceil(cols);
    let d = Diagram {
        width: PAD + cols as f32 * (BOX_W + GAP_X) - GAP_X + PAD,
        height: PAD + rows as f32 * (BOX_H + GAP_Y) - GAP_Y + PAD,
        ..Diagram::default()
    };
    Scene {
        diagram: d,
        boxes,
        links,
        hubs,
    }
}

/// The stub for relations from the module's entities to models outside the
/// sources, placed after `n` stubs in the row at `y`.
fn external_stub(model: &DomainModel, inside: &BTreeSet<&str>, n: usize, y: f32) -> Option<ModBox> {
    let count = (model.relations.iter())
        .filter(|r| r.external && inside.contains(r.from.as_str()))
        .count();
    (count > 0).then(|| ModBox {
        module: usize::MAX,
        title: format!("\u{2192} external ({count})"),
        sub: String::new(),
        x: PAD + n as f32 * (STUB_W + 16.0),
        y,
        w: STUB_W,
        h: STUB_H,
        external: true,
    })
}

fn of_module(model: &DomainModel, module: &str, pairs: &[(usize, usize)], hubs: Vec<Hub>) -> Scene {
    let keep: Vec<usize> = (0..model.entities.len())
        .filter(|&i| model.entities[i].module == module)
        .collect();
    let inside: BTreeSet<&str> = keep
        .iter()
        .map(|&i| model.entities[i].id.as_str())
        .collect();
    let sub = DomainModel {
        entities: keep.iter().map(|&i| model.entities[i].clone()).collect(),
        relations: (model.relations.iter())
            .filter(|r| inside.contains(r.from.as_str()) && inside.contains(r.to.as_str()))
            .cloned()
            .collect(),
        ..DomainModel::default()
    };
    let mut d = entities::layout(&sub);
    d.cards.iter_mut().for_each(|c| c.entity = keep[c.entity]);
    d.bands
        .iter_mut()
        .for_each(|b| b.root = b.root.map(|r| keep[r]));
    let names = modules(model);
    let hub = |i: usize| hubs.iter().any(|h| h.entity == i);
    let mut foreign: BTreeMap<usize, usize> = BTreeMap::new();
    for &(a, b) in pairs.iter().filter(|(_, b)| !hub(*b)) {
        let (x, y) = (&model.entities[a].module, &model.entities[b].module);
        if x != y && (x == module || y == module) {
            let other = if x == module { y } else { x };
            *foreign
                .entry(names.iter().position(|n| n == other).unwrap_or(0))
                .or_default() += 1;
        }
    }
    let mut stubs: Vec<(usize, usize)> = foreign.into_iter().collect();
    stubs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    // One row below the bands.
    let y = d.height;
    let mut boxes: Vec<ModBox> = (stubs.iter().enumerate())
        .map(|(k, &(m, n))| ModBox {
            module: m,
            title: format!("\u{2192} {} ({n})", names[m]),
            sub: String::new(),
            x: PAD + k as f32 * (STUB_W + 16.0),
            y,
            w: STUB_W,
            h: STUB_H,
            external: false,
        })
        .collect();
    boxes.extend(external_stub(model, &inside, boxes.len(), y));
    if !boxes.is_empty() {
        d.width = d.width.max(PAD + boxes.len() as f32 * (STUB_W + 16.0));
        d.height = y + STUB_H + PAD;
    }
    let hubs = hubs
        .into_iter()
        .filter(|h| model.entities[h.entity].module == module)
        .collect();
    Scene {
        diagram: d,
        boxes,
        links: vec![],
        hubs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{Entity, Relation, RelationKind};

    fn entity(name: &str, module: &str, kind: EntityKind) -> Entity {
        Entity {
            id: name.into(),
            name: name.into(),
            kind,
            fields: vec![],
            file: format!("{module}/{name}.py"),
            line: 1,
            source: "t".into(),
            module: module.into(),
        }
    }

    fn rel(from: &str, to: &str) -> Relation {
        Relation {
            from: from.into(),
            to: to.into(),
            kind: RelationKind::HasOne,
            via: to.into(),
            source: "t".into(),
            external: false,
        }
    }

    /// No aggregates: dcim has Device, Site, Rack; ipam Prefix, Vlan; tenancy
    /// Tenant, targeted from dcim, ipam and users (a hub); extras links.
    fn netbox() -> DomainModel {
        use EntityKind::Entity as E;
        let e = |n, m| entity(n, m, E);
        DomainModel {
            entities: vec![
                e("Device", "dcim"),
                e("Site", "dcim"),
                e("Rack", "dcim"),
                e("Prefix", "ipam"),
                e("Vlan", "ipam"),
                e("Tenant", "tenancy"),
                e("Token", "users"),
            ],
            relations: vec![
                rel("Device", "Site"),
                rel("Device", "Rack"),
                rel("Device", "Tenant"),
                rel("Prefix", "Tenant"),
                rel("Token", "Tenant"),
                rel("Prefix", "Site"),
                rel("Vlan", "Site"),
                rel("Vlan", "Prefix"),
            ],
            ..DomainModel::default()
        }
    }

    #[test]
    fn overview_when_several_modules_and_no_aggregates_or_many_entities() {
        let mut m = netbox();
        assert!(wanted(&m));
        m.entities[0].kind = EntityKind::AggregateRoot;
        assert!(!wanted(&m), "aggregates and few entities: plain bands");
        for i in 0..OVERVIEW_MIN_ENTITIES {
            m.entities
                .push(entity(&format!("X{i}"), "dcim", EntityKind::Entity));
        }
        assert!(wanted(&m), "many entities even with aggregates");
        let one: Vec<_> = m
            .entities
            .iter()
            .cloned()
            .map(|mut e| {
                e.module = "a".into();
                e
            })
            .collect();
        assert!(!wanted(&DomainModel { entities: one, ..m }), "one module");
    }

    #[test]
    fn module_boxes_count_entities_and_name_the_top_three() {
        let s = scene(&netbox(), None);
        let b: Vec<(&str, &str)> = s
            .boxes
            .iter()
            .map(|b| (b.title.as_str(), b.sub.as_str()))
            .collect();
        assert_eq!(
            b[0],
            ("dcim (3)", "Device, Site, Rack"),
            "most related first"
        );
        assert_eq!(b[1].0, "ipam (2)");
        assert!(s.diagram.cards.is_empty());
    }

    #[test]
    fn module_arrows_are_bundled_and_hubs_become_badges() {
        let s = scene(&netbox(), None);
        let names = modules(&netbox());
        let tenant = netbox()
            .entities
            .iter()
            .position(|e| e.name == "Tenant")
            .unwrap();
        assert_eq!(
            s.hubs,
            [Hub {
                entity: tenant,
                from: 3
            }]
        );
        // ipam -> dcim: Prefix->Site and Vlan->Site; Tenant relations are no arrows.
        assert_eq!(names[..2], ["dcim", "ipam"]);
        let counts: Vec<usize> = s.links.iter().map(|l| l.count).collect();
        assert_eq!(counts, [2]);
    }

    #[test]
    fn two_modules_are_no_hub() {
        let mut m = netbox();
        m.relations.retain(|r| r.from != "Token");
        assert!(scene(&m, None).hubs.is_empty());
    }

    #[test]
    fn drill_down_keeps_one_module_with_a_stub_per_foreign_module() {
        let m = netbox();
        let s = scene(&m, Some("ipam"));
        let cards: Vec<&str> = s
            .diagram
            .cards
            .iter()
            .map(|c| m.entities[c.entity].name.as_str())
            .collect();
        assert_eq!(cards.len(), 2);
        assert!(cards.contains(&"Prefix") && cards.contains(&"Vlan"));
        let stubs: Vec<&str> = s.boxes.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(
            stubs,
            ["\u{2192} dcim (2)"],
            "Tenant relations are the hub badge"
        );
        assert_eq!(s.diagram.arrows.len(), 1, "Vlan -> Prefix stays");
        assert!(s.hubs.is_empty(), "the hub lives in tenancy");
        let t = scene(&m, Some("tenancy"));
        assert_eq!(t.hubs.len(), 1);
    }

    #[test]
    fn a_small_model_keeps_the_plain_diagram() {
        let m = DomainModel {
            entities: vec![
                entity("Order", "a", EntityKind::AggregateRoot),
                entity("Buyer", "b", EntityKind::AggregateRoot),
            ],
            relations: vec![rel("Order", "Buyer")],
            ..DomainModel::default()
        };
        let (s, plain) = (scene(&m, None), entities::layout(&m));
        assert!(s.boxes.is_empty() && s.hubs.is_empty());
        assert_eq!(s.diagram.cards, plain.cards);
        assert_eq!(s.diagram.bands, plain.bands);
    }

    #[test]
    fn module_view_counts_external_relations_in_one_stub() {
        let mut m = netbox();
        let mut r = rel("Prefix", "django.contrib.auth.User");
        r.external = true;
        m.relations.push(r);
        let s = scene(&m, Some("ipam"));
        let stubs: Vec<&str> = s.boxes.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(stubs, ["\u{2192} dcim (2)", "\u{2192} external (1)"]);
        assert!(s.boxes[1].external && !s.boxes[0].external);
    }
}
