//! The connection entries of the architecture World's details: open and
//! closed by their ends, each leading to its neighbour area
//! (`worldDetails`' evidence list in `viewer_world.js`).

use super::state::{Selection, StartState};

impl StartState {
    /// The ends of connection `i` of the scope, by area name.
    fn connection_key(&self, i: usize) -> Option<(String, String)> {
        let c = self.scope.connections.get(i)?;
        let name = |a: usize| self.scope.areas[a].name.clone();
        Some((name(c.from), name(c.to)))
    }

    /// A connection entry of the details was clicked: open or close it.
    pub(super) fn toggle_entry(&mut self, i: usize) {
        if self.services_mode() {
            self.with_services(|w| {
                w.toggle_link(i);
                false
            });
        } else {
            self.toggle_connection(i);
        }
    }

    /// Open or close the entry of connection `i`.
    pub(super) fn toggle_connection(&mut self, i: usize) {
        let Some(key) = self.connection_key(i) else {
            return;
        };
        if !self.open_connections.remove(&key) {
            self.open_connections.insert(key);
        }
    }

    pub(super) fn connection_open(&self, i: usize) -> bool {
        self.connection_key(i)
            .is_some_and(|k| self.open_connections.contains(&k))
    }

    /// "Select neighbor": the other area of a connection, details open.
    pub(super) fn select_neighbor(&mut self, area: usize) {
        if area < self.scope.areas.len() {
            self.selection = Some(Selection {
                area,
                entity: None,
                members: None,
            });
            self.limit = 20;
        }
    }
}
