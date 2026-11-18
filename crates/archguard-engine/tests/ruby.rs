//! Ruby import graph (ADR 0019): `require_relative`, `require` against
//! `lib/`, gems as external, and Zeitwerk constant references only in a
//! Rails app.

use std::collections::BTreeSet;
use std::fs;

use archguard_core::Config;
use archguard_engine::Level;

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    dir
}

/// Every edge as (from, to) node names, and the warnings. The derived
/// source root (`lib`, `app`) is not part of the names.
fn graph(files: &[(&str, &str)]) -> (BTreeSet<(String, String)>, Vec<String>) {
    let dir = project(files);
    let loaded = archguard_engine::load_config(None, dir.path()).expect("config");
    let run =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, Level::File).expect("analyze");
    let (graph, warnings) = (run.graph, run.warnings);
    let name = |id| {
        graph
            .nodes
            .iter()
            .find(|node| node.id == id)
            .unwrap()
            .name
            .clone()
    };
    let edges = graph
        .edges
        .iter()
        .map(|edge| (name(edge.from), name(edge.to)))
        .collect();
    (edges, warnings)
}

fn edge(from: &str, to: &str) -> (String, String) {
    (from.to_string(), to.to_string())
}

#[test]
fn requires_resolve_to_files_and_gems_stay_external() {
    let (edges, warnings) = graph(&[
        (
            "lib/shop.rb",
            "require 'json'\nrequire 'shop/cart'\nrequire_relative 'shop/tax'\n\nmodule Shop\nend\n",
        ),
        (
            "lib/shop/cart.rb",
            "require_relative '../shop'\n\nmodule Shop\n  class Cart\n    def total\n      Tax.rate\n    end\n  end\nend\n",
        ),
        (
            "lib/shop/tax.rb",
            "module Shop\n  class Tax\n    def self.rate = 0.19\n  end\nend\n",
        ),
    ]);
    for expected in [
        edge("shop.rb", "shop/cart.rb"),
        edge("shop.rb", "shop/tax.rb"),
        edge("shop/cart.rb", "shop.rb"),
        // Not through Zeitwerk (no Rails app) but through the resolved call
        // `Tax.rate` on a unique project class.
        edge("shop/cart.rb", "shop/tax.rb"),
    ] {
        assert!(
            edges.contains(&expected),
            "missing {expected:?} in {edges:?}"
        );
    }
    assert_eq!(edges.len(), 4, "{edges:?}");
    assert!(
        warnings.iter().any(|w| w.contains("`json`")),
        "{warnings:?}"
    );
}

#[test]
fn constant_references_follow_zeitwerk_only_in_a_rails_app() {
    let files = [
        (
            "app/controllers/orders_controller.rb",
            "class OrdersController < ApplicationController\n  def show\n    @order = Billing::Invoice.find(params[:id])\n  end\nend\n",
        ),
        (
            "app/controllers/application_controller.rb",
            "class ApplicationController < ActionController::Base\nend\n",
        ),
        (
            "app/models/billing/invoice.rb",
            "module Billing\n  class Invoice < ApplicationRecord\n    include Payable\n  end\nend\n",
        ),
        (
            "app/models/application_record.rb",
            "class ApplicationRecord < ActiveRecord::Base\n  self.abstract_class = true\nend\n",
        ),
        ("app/models/concerns/payable.rb", "module Payable\nend\n"),
    ];
    // Without config/application.rb the constants stay unresolved; the
    // superclasses still link through the call analysis.
    let (plain, warnings) = graph(&files);
    assert!(
        !plain.contains(&edge(
            "controllers/orders_controller.rb",
            "models/billing/invoice.rb"
        )),
        "{plain:?}"
    );
    assert!(
        warnings.iter().all(|w| !w.contains("Billing")),
        "constant references never warn: {warnings:?}"
    );
    let mut rails = files.to_vec();
    rails.push((
        "config/application.rb",
        "require_relative 'boot'\nmodule Shop\n  class Application < Rails::Application\n  end\nend\n",
    ));
    let (edges, _) = graph(&rails);
    for expected in [
        edge(
            "controllers/orders_controller.rb",
            "models/billing/invoice.rb",
        ),
        edge(
            "controllers/orders_controller.rb",
            "controllers/application_controller.rb",
        ),
        edge("models/billing/invoice.rb", "models/application_record.rb"),
        // `app/models/concerns` is an autoload root of its own.
        edge("models/billing/invoice.rb", "models/concerns/payable.rb"),
    ] {
        assert!(
            edges.contains(&expected),
            "missing {expected:?} in {edges:?}"
        );
    }
}

#[test]
fn ruby_classes_are_measured_on_their_graph_nodes() {
    let dir = project(&[(
        "lib/shop/cart.rb",
        "module Shop\n  class Cart\n    def total(items)\n      items.sum { |i| i.price if i }\n    end\n  end\nend\n",
    )]);
    let report = archguard_engine::analyze_calls(dir.path(), &Config::default());
    let analyzer = report
        .analyzers
        .iter()
        .find(|a| a.language == "ruby")
        .expect("ruby analyzer");
    assert_eq!(
        (
            analyzer.backend.as_str(),
            analyzer.status.as_str(),
            analyzer.files
        ),
        ("archguard tokens", "partial", 1)
    );
    let cart = report
        .classes
        .iter()
        .find(|c| c.name == "Shop::Cart")
        .expect("class");
    assert_eq!(cart.id, "ruby:lib/shop/cart.rb::Shop::Cart");
    assert_eq!(cart.graph_node, "lib/shop/cart.rb");
    let total = report.methods.iter().find(|m| m.name == "total").unwrap();
    // The modifier `if` inside the block: one decision.
    assert_eq!(total.complexity.as_ref().unwrap().cyclomatic, 2);
}
