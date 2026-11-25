//! The index page model as Slint data (`es_index_page.slint`).

use super::state::EsState;
use crate::ui::EsPage;

/// The focused node's page: indices, templates and undeclared indices have one.
pub fn page(_s: &EsState) -> Option<EsPage> {
    None
}
