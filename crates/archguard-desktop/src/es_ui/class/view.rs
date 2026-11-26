//! View wiring to Slint, ticket 12: the one place where the class models become Slint data.

use crate::es_ui::state::EsState;
use crate::ui::EsClassModel;

/// Stub (red commit): the model is empty.
pub fn build(_s: &mut EsState) -> EsClassModel {
    EsClassModel::default()
}

pub fn fit(_s: &mut EsState, _w: f32, _h: f32) {}
