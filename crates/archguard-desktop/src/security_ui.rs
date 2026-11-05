//! Security tab. Not native yet: the view shows the web-viewer fallback.
//! Implement it here and in `ui/security_view.slint` (`SecurityData` global).

use crate::context::Loaded;
use crate::ui::AppWindow;

/// Register the tab's callbacks once at start-up.
pub fn wire(_ui: &AppWindow) {}

/// Fill the tab after an analysis finished.
pub fn show(_ui: &AppWindow, _data: &Loaded) {}
