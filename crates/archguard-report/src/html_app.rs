//! The single-page HTML application embedded by [`crate::to_html`] (T39).
//!
//! [`TEMPLATE`] is the page skeleton (CSS, DOM markers, placeholders
//! `__APP_JS__`, `__CDN__`, `__PAYLOAD__`); [`APP_JS`] is the vanilla-JS
//! client. They live apart from `html.rs` only so no single file grows
//! past a reviewable size — the delivered page is still ONE file.
//!
//! The JS deliberately mirrors server-side Rust so the browser shows
//! exactly what the CLI computes at the same zoom level (each JS section
//! names the Rust function it mirrors). The browser NEVER edits code:
//! the repair mode only records SimOps in the frozen T37 wire format
//! (see `archguard-analysis`'s `simulate.rs` module docs) for export.

/// Assets are embedded at compile time; exported reports remain standalone.
pub(crate) const TEMPLATE: &str = include_str!("viewer.html");
pub(crate) const APP_JS: &str = concat!(
    include_str!("viewer_scc.js"),
    "\n",
    include_str!("viewer_i18n.js"),
    "\n",
    include_str!("cycle_planner.js"),
    "\n",
    include_str!("viewer_review.js"),
    "\n",
    include_str!("viewer_quality_metrics.js"),
    "\n",
    include_str!("viewer_quality_findings.js"),
    "\n",
    include_str!("viewer_detail_graphs.js"),
    "\n",
    include_str!("viewer_class_heatmap.js"),
    "\n",
    include_str!("viewer_engineering_quality.js"),
    "\n",
    include_str!("viewer_dependencies.js"),
    "\n",
    include_str!("viewer_tree_layout.js"),
    "\n",
    include_str!("viewer_tree.js"),
    "\n",
    include_str!("viewer_panels.js"),
    "\n",
    include_str!("viewer_services.js"),
    "\n",
    include_str!("viewer_workspace.js"),
    "\n",
    include_str!("viewer_export.js"),
    "\n",
    include_str!("viewer_patterns.js"),
    "\n",
    include_str!("viewer_architecture.js"),
    "\n",
    include_str!("viewer_assessment.js"),
    "\n",
    include_str!("viewer_calls_tree.js"),
    "\n",
    include_str!("viewer_call_rankings.js"),
    "\n",
    include_str!("viewer_method_metrics.js"),
    "\n",
    include_str!("viewer_call_rankings_ui.js"),
    "\n",
    include_str!("viewer_call_cycles_model.js"),
    "\n",
    include_str!("viewer_call_cycles_ui.js"),
    "\n",
    include_str!("viewer_security.js"),
    "\n",
    include_str!("viewer_dash_kpis.js"),
    "\n",
    include_str!("viewer_dash_zones.js"),
    "\n",
    include_str!("viewer_dash_health.js"),
    "\n",
    include_str!("viewer_repository.js"),
    "\n",
    include_str!("viewer_world_model.js"),
    include_str!("viewer_world.js"),
    include_str!("viewer_development.js"),
    "\n",
    include_str!("viewer.js"),
    "\n",
    include_str!("viewer_scenarios.js")
);
