//! The command-line interface as clap sees it.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Default location of the graph-DB mirror of `db sync|query`
/// (Kap. 8, T33).
pub(crate) const DEFAULT_DB_PATH: &str = ".archguard/graph.db";

/// Default row cap of `db query` (mirrors the MCP's `DEFAULT_LIMIT`).
pub(crate) const DEFAULT_DB_LIMIT: usize = 20;

#[derive(Parser)]
#[command(
    name = "archguard",
    version,
    about = "Analyze software architecture and dependencies."
)]
pub(crate) struct Cli {
    /// Config file; defaults to <project>/.archguard/archguard.toml, then legacy archguard.toml.
    #[arg(long, global = true, value_name = "PFAD")]
    pub(crate) config: Option<PathBuf>,
    /// Output format. `mermaid` is only valid for `cycles`, `sarif`
    /// only for `check`; `graph` has its own `--format` (dot, mermaid,
    /// json, html, graphml, gexf).
    #[arg(
        long,
        global = true,
        value_name = "FORMAT",
        value_enum,
        default_value = "text"
    )]
    pub(crate) format: Format,
    /// Suppress summary and finding output on stdout.
    #[arg(long, short = 'q', global = true)]
    pub(crate) quiet: bool,
    /// Extra diagnostics on stdout.
    #[arg(long, short = 'v', global = true)]
    pub(crate) verbose: bool,
    #[command(subcommand)]
    pub(crate) command: Command,
}

/// Global `--format` value set. `dot`, `html`, `graphml` and `gexf`
/// exist only for `graph`, `table` only for `db query` and `trend`; these
/// commands re-declare `--format` as their own (subcommand-level)
/// argument, with the defaults `dot` and `table`. Every other command
/// rejects the foreign values at runtime, like `mermaid`/`sarif` outside
/// their home commands. (The re-declared argument must share this type: clap keeps
/// one arg per id, and the global's parent-side downcast would break on
/// a foreign enum.)
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub(crate) enum Format {
    Text,
    Json,
    Mermaid,
    Sarif,
    Dot,
    Html,
    Graphml,
    Gexf,
    Table,
    /// Markdown PR comment against `review --base`.
    PrComment,
}

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub(crate) enum LevelArg {
    Module,
    File,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Assemble architecture findings, change impact, evidence gaps and review decisions.
    Review {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Compare directly with this Git commit/ref under current policy.
        #[arg(long)]
        base: Option<String>,
        /// Import an explicit decision document exported from this review revision.
        #[arg(long, value_name = "JSON")]
        decisions: Option<PathBuf>,
        /// Record an immutable review and update .archguard/review.json.
        #[arg(long)]
        save: bool,
        /// With `--format pr-comment`: write the Markdown to this path
        /// instead of stdout; bare `--out` uses
        /// .archguard/reports/pr-comment.md.
        #[arg(
            long,
            value_name = "DATEI",
            num_args = 0..=1,
            default_missing_value = ".archguard/reports/pr-comment.md"
        )]
        out: Option<PathBuf>,
    },
    /// Record and inspect review decisions (`.archguard/reviews.json`).
    ReviewDecision {
        #[command(subcommand)]
        action: ReviewDecisionAction,
    },
    /// Extract the graph, write it as JSON (`--out`) and print a summary.
    Scan {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Override graph JSON output (default: <project>/.archguard/graph.json).
        #[arg(long, value_name = "DATEI")]
        out: Option<PathBuf>,
        /// Graph granularity.
        #[arg(long, value_enum, default_value = "module")]
        level: LevelArg,
        /// Also measure the Martin A/I zones of the review KPIs: runs the
        /// call analysis with compiler helpers (javac, .NET), so it is slow.
        /// Zones stay "not measured" without Java/C# abstractness.
        #[arg(long)]
        measure_zones: bool,
    },
    /// Report cycles.
    Cycles {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Graph granularity.
        #[arg(long, value_enum, default_value = "module")]
        level: LevelArg,
        /// Show at most N cycle findings.
        #[arg(long, value_name = "N")]
        max_cycles: Option<usize>,
        /// Exit with code 1 when cycles are found.
        #[arg(long)]
        fail_on_found: bool,
    },
    /// The CI command: cycles + layer rules + thresholds against the
    /// config.
    Check {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Accept the cycles recorded in this baseline; only NEW cycles
        /// fail. Bare `--baseline` uses `[rules].baseline` under the
        /// project root.
        #[arg(long, value_name = "DATEI", num_args = 0..=1)]
        baseline: Option<Option<PathBuf>>,
        /// Compare the working tree to this Git ref under the current policy.
        #[arg(long)]
        base: Option<String>,
        /// Fail when ACD grows by more than this percentage (bare flag: 10).
        /// Reference: --base, otherwise the latest saved snapshot.
        #[arg(long, num_args=0..=1, default_missing_value="10")]
        fail_on_acd_regression: Option<f64>,
        /// Also measure the Martin A/I zones of the review KPIs: runs the
        /// call analysis with compiler helpers (javac, .NET), so it is slow.
        /// Zones stay "not measured" without Java/C# abstractness.
        #[arg(long)]
        measure_zones: bool,
    },
    /// Scan dependency advisories with Trivy and save .archguard/security.json.
    Security {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Read the saved snapshot without executing Trivy or accessing the network.
        #[arg(long, conflicts_with_all = ["offline", "cache_dir"])]
        cached: bool,
        /// Use the existing local Trivy database without downloading updates.
        #[arg(long)]
        offline: bool,
        /// Override Trivy's database cache directory.
        #[arg(long, value_name = "PFAD")]
        cache_dir: Option<PathBuf>,
        /// Exit 1 for advisories; exit 3 if the scan is missing, stale or incomplete.
        #[arg(long)]
        fail_on_found: bool,
    },
    /// Coupling metrics per module.
    Metrics {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Include class/method quality, Git knowledge concentration and measured runs.
        #[arg(long, conflicts_with_all = ["module", "top"])]
        extended: bool,
        /// Import revision-labelled build, runtime or repeated-test observations.
        #[arg(long, requires = "extended", value_name = "JSON")]
        report: Option<PathBuf>,
        /// Show only this module.
        #[arg(long, value_name = "NAME")]
        module: Option<String>,
        /// Show only the N most unstable modules.
        #[arg(long, value_name = "N")]
        top: Option<usize>,
    },
    /// Ranked, explained review candidates from coupling, cycles and Git churn.
    Hotspots {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "module")]
        level: LevelArg,
        #[arg(long, default_value_t = 20)]
        top: usize,
        /// Git history window, e.g. 180.days or 6.months.
        #[arg(long)]
        since: Option<String>,
    },
    /// Find repeatedly co-changing source pairs without static import edges.
    CoChange {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long, default_value_t = 3)]
        min_shared: u32,
        #[arg(long, default_value_t = 0.5)]
        min_ratio: f64,
        #[arg(long, default_value_t = 50)]
        top: usize,
    },
    /// Explain CODEOWNERS boundaries and configured forbidden team directions.
    Ownership {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Structural load, heavy imports, barrels and compiler-backed abstractions.
    Performance {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Compiler-backed classes, methods, calls and per-language coverage.
    Calls {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Structural cohesion of immediate package members and split-review candidates.
    Cohesion {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Compiler-backed class cohesion (LCOM4), groups and instance complexity.
        #[arg(long)]
        classes: bool,
    },
    /// Read quality history, or explicitly record current per-package measurements.
    QualityHistory {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long)]
        record: bool,
        #[arg(long, requires = "record")]
        tag: Option<String>,
    },
    /// Import JUnit test results, optional exact source mappings, and recommend affected tests.
    TestEvidence {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long, value_name = "XML")]
        report: Option<PathBuf>,
        #[arg(long, requires = "report", value_name = "JSON")]
        mapping: Option<PathBuf>,
        #[arg(long, requires = "mapping", value_name = "PFAD")]
        source_root: Option<PathBuf>,
        /// Exact graph-node or mapped source path; repeat for multiple changed targets.
        #[arg(long, value_name = "FILE")]
        target: Vec<String>,
    },
    /// Import or read mutation-testing-elements JSON evidence without executing tests.
    Mutations {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long, value_name = "JSON")]
        report: Option<PathBuf>,
        #[arg(long, requires = "report", value_name = "PFAD")]
        source_root: Option<PathBuf>,
    },
    /// Compare findings with an expert assessment: recall, precision, unjudged findings (ADR 0005).
    Evaluate {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Assessment file (default: <project>/.archguard/assessment.toml).
        #[arg(long, value_name = "DATEI")]
        assessment: Option<PathBuf>,
        /// Write a template with every current finding as an unjudged stub; never overwrites.
        #[arg(long)]
        init: bool,
    },
    /// Assess architecture pattern candidates from original graph, compiler and contract evidence.
    Patterns {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Import CPD XML or run the built-in token scan, or read saved duplicate groups.
    Duplicates {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long, value_name = "XML", conflicts_with = "scan")]
        report: Option<PathBuf>,
        #[arg(long, requires = "report", value_name = "PFAD")]
        source_root: Option<PathBuf>,
        /// Run ArchGuard's token clone scanner over the walked sources instead of importing CPD.
        #[arg(long)]
        scan: bool,
        /// Shortest reported clone in lexical tokens (one punctuation character is one token).
        #[arg(long, requires = "scan", default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..))]
        min_tokens: u32,
        /// Also match copies whose identifiers and literals differ (Type-2 clones).
        #[arg(long, requires = "scan")]
        normalize: bool,
        /// Overwrite an imported CPD snapshot; without it the scan refuses.
        #[arg(long, requires = "scan")]
        replace: bool,
    },
    /// Import an OpenAPI contract for an exact service ID, or read saved comparisons.
    Contracts {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        #[arg(long, requires = "service", value_name = "JSON_OR_YAML")]
        report: Option<PathBuf>,
        #[arg(long, requires = "report", value_name = "ID")]
        service: Option<String>,
        #[arg(long, requires = "report", value_name = "JSON")]
        usage: Option<PathBuf>,
    },
    /// Import test coverage, or read its saved snapshot and change-impact ranking.
    Coverage {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// JaCoCo/Cobertura/Clover XML or LCOV tracefile; relative paths are resolved under the project.
        #[arg(long, value_name = "DATEI")]
        report: Option<PathBuf>,
        /// Project-relative source prefix for report paths (e.g. src/main/java).
        #[arg(long, requires = "report", value_name = "PFAD")]
        source_root: Option<PathBuf>,
    },
    /// Check which external packages this OS (macOS/Windows/Linux) needs for
    /// full features: git history, Java/C#/TypeScript call analysis. Always exit 0.
    Doctor,
    /// Replace this binary with the latest published GitHub release after
    /// verifying its SHA-256 checksums.
    Update {
        /// Only report whether a newer release exists; change nothing.
        #[arg(long)]
        check: bool,
    },
    /// Import/read SARIF quality findings, optionally gate changed-code quality.
    Quality {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Explicit SARIF 2.1.0 import; without this flag, read the saved findings.
        #[arg(long, value_name = "SARIF")]
        report: Option<PathBuf>,
        /// Project-relative source prefix for report paths.
        #[arg(long, requires = "report", value_name = "PFAD")]
        source_root: Option<PathBuf>,
        /// Compare working-tree source lines with this Git ref. Requires a threshold.
        #[arg(long)]
        base: Option<String>,
        /// Minimum coverage of changed executable lines, 0..=100 (import coverage first).
        #[arg(long, requires = "base", value_name = "PERCENT")]
        min_changed_coverage: Option<f64>,
        /// Maximum AST complexity of methods intersecting changed lines, at least 1.
        #[arg(long, requires = "base", value_name = "N")]
        max_complexity: Option<u32>,
        /// Maximum control-structure nesting of methods intersecting changed lines.
        #[arg(long, requires = "base", value_name = "N")]
        max_nesting: Option<u32>,
    },
    /// Compare modeled logical components and allowed directions with actual imports.
    Architecture {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Discover monorepo projects, applications and Compose HTTP/startup dependency cycles.
    Services {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Exit 1 for configured HTTP cycles; startup cycles are reported separately.
        #[arg(long)]
        fail_on_found: bool,
    },
    /// Runtime profile from recorded OTLP traces in .archguard/runtime (ADR 0004).
    Runtime {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Analyze declared dependencies between local projects; source analysis is opt-in.
    Workspace {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Include isolated source graphs and supported compiler-resolved cross-project calls.
        #[arg(long)]
        sources: bool,
        /// Project ID, qualified file ID or repository-relative source path to inspect.
        #[arg(long, value_name = "TARGET", requires = "sources")]
        target: Vec<String>,
    },
    /// Generate architecture Markdown, or fail if the saved document has drifted.
    Doc {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// File relative to the project root, or an absolute path.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Generate/update arc42.md with protected manual text and 12 evidence chapters.
        #[arg(long)]
        arc42: bool,
        /// Only update/export/check this arc42 chapter (1..=12).
        #[arg(long, requires = "arc42", value_parser = clap::value_parser!(u8).range(1..=12))]
        chapter: Option<u8>,
        /// Print Markdown without writing a file.
        #[arg(long, conflicts_with_all = ["check", "out"])]
        stdout: bool,
        /// Read-only comparison; exit 1 if the document is missing or outdated.
        #[arg(long)]
        check: bool,
    },
    /// Compare two Git refs, or a ref with the staged index; never the worktree.
    Diff {
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Explicitly select the Git index as target.
        #[arg(long, required_unless_present = "to", conflicts_with = "to")]
        staged: bool,
        /// Left-hand branch, tag or commit.
        #[arg(long = "from", visible_alias = "base", default_value = "HEAD")]
        base: String,
        /// Right-hand branch, tag or commit; read directly without a checkout.
        #[arg(long, required_unless_present = "staged")]
        to: Option<String>,
        #[arg(long, value_enum, default_value = "html")]
        format: Format,
    },
    /// Export the graph to stdout and <project>/.archguard/reports/.
    Graph {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Export format (default: dot).
        #[arg(long, value_enum, default_value = "dot")]
        format: Format,
        /// Restrict the export to this module's neighbourhood.
        #[arg(long, value_name = "MODUL")]
        focus: Option<String>,
        /// Radius around `--focus` to include (default 1 when focusing).
        #[arg(long, value_name = "N")]
        depth: Option<u32>,
        /// Zoom into these modules' children first (comma-separated or
        /// repeated) — the aggregate view the analysis checked, with
        /// these modules unfolded.
        #[arg(long, value_name = "MODULN", value_delimiter = ',')]
        expand: Vec<String>,
    },
    /// Serve a live graph on localhost; source/config changes update the browser.
    Serve {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Loopback port; use 0 to choose an available port.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Initial hierarchy depth; 0 shows top-level packages.
        #[arg(long, default_value_t = 0)]
        depth: u32,
    },
    /// Dependency-structure matrix of the module view.
    Dsm {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Zoom into these modules' children first (comma-separated or
        /// repeated), like `graph --expand`.
        #[arg(long, value_name = "MODULN", value_delimiter = ',')]
        expand: Vec<String>,
        /// Show exactly the direct children of this module.
        #[arg(long, value_name = "MODUL")]
        within: Option<String>,
        /// Output format (default: text).
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Simulate virtual refactorings (cut/move/merge ops) on the module
    /// view and print the delta without changing the graph on disk.
    Simulate {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Cut the edge `VON,BIS` (repeatable).
        #[arg(long, value_name = "VON,BIS")]
        cut: Vec<String>,
        /// Move node `KNOTEN` under parent `ELTERN` (repeatable).
        #[arg(long = "move", value_name = "KNOTEN:ELTERN")]
        move_node: Vec<String>,
        /// Merge `UEBERLEBENDER,AUFGENOMMENER` (repeatable).
        #[arg(long, value_name = "UEBERLEBENDER,AUFGENOMMENER")]
        merge: Vec<String>,
        /// JSON file containing operations exported by the viewer with "Export SimOps".
        /// Applied after operations supplied as command-line flags.
        #[arg(long, value_name = "DATEI")]
        ops: Option<PathBuf>,
        /// Output format (default: text).
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Cut suggestions for cycles.
    Suggest {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Only this cycle finding, e.g. `scc-1`.
        #[arg(long, value_name = "ID")]
        cycle: Option<String>,
        /// Show at most N cuts per cycle.
        #[arg(long, value_name = "N")]
        max: Option<usize>,
    },
    /// Baseline ratchet: accept existing cycles, block new ones.
    Baseline {
        #[command(subcommand)]
        action: BaselineAction,
    },
    /// Write .archguard/archguard.toml; refuses to overwrite or shadow existing config.
    Init {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Write the commented stock defaults instead of the derived config.
        #[arg(long)]
        defaults: bool,
    },
    /// Show the effective config with per-section provenance.
    Config {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
    },
    /// Append a snapshot of the current summary to
    /// `.archguard/history/` for the trend.
    Snapshot {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Free label stored inside the snapshot file; the file name
        /// keeps the timestamp and the git SHA (`notag` without git).
        #[arg(long, value_name = "TAG")]
        tag: Option<String>,
    },
    /// Print the snapshot history — fixed-width table with `+`/`-`
    /// deltas against the previous row, or JSON.
    Trend {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Show only the N newest snapshots (0 = all).
        #[arg(long, value_name = "N", default_value_t = 0)]
        last: usize,
        /// Output format (default: table).
        #[arg(long, value_enum, default_value = "table")]
        format: Format,
    },
    /// Store the analysis in a graph database and query it with openCypher.
    /// Requires an ArchGuard installation with database support.
    Db {
        #[command(subcommand)]
        action: DbAction,
    },
    /// Start the MCP server on stdio.
    Mcp {
        /// Project root the tools analyse by default.
        #[arg(long, value_name = "PFAD", default_value = ".")]
        root: PathBuf,
    },
}

/// The decision kinds of `review-decision record`.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
pub(crate) enum DecisionArg {
    Fix,
    Defer,
    AcceptRisk,
}

/// Subcommands of `review-decision`; the handler lives in
/// [`review_session_cmd`].
#[derive(Subcommand)]
pub(crate) enum ReviewDecisionAction {
    /// Record a decision about a finding (re-recording the same id replaces it).
    Record {
        /// Stable finding id, e.g. `scc-3`.
        #[arg(long)]
        finding: String,
        /// What was decided about the finding.
        #[arg(long, value_enum)]
        decision: DecisionArg,
        /// Architecture decision record (ADR) link explaining the decision.
        #[arg(long, value_name = "PFAD")]
        adr: Option<PathBuf>,
        /// Free-form rationale.
        #[arg(long)]
        note: Option<String>,
        /// Recorder (default: cli).
        #[arg(long)]
        author: Option<String>,
        /// Decision id (default: the finding id).
        #[arg(long)]
        id: Option<String>,
    },
    /// List all recorded decisions.
    List,
    /// Show one decision by id (or by finding id).
    Show {
        /// Decision id, falling back to the finding id.
        id: String,
    },
}

/// Subcommands of `db`; the handler lives in [`db`].
#[derive(Subcommand)]
pub(crate) enum DbAction {
    /// Analyse the project and mirror graph and report into the DB.
    Sync {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Path of the graph database.
        #[arg(long, value_name = "PFAD", default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
    },
    /// Run a read-only openCypher statement against the DB.
    Query {
        /// The statement, e.g. "MATCH (m:module) RETURN m.name".
        cypher: String,
        /// Path of the graph database.
        #[arg(long, value_name = "PFAD", default_value = DEFAULT_DB_PATH)]
        db: PathBuf,
        /// Maximum number of result rows. Database resource limits still apply.
        #[arg(long, value_name = "N", default_value_t = DEFAULT_DB_LIMIT)]
        limit: usize,
        /// Output format: table (default) or json.
        #[arg(long, value_enum, default_value = "table")]
        format: Format,
    },
}

#[derive(Subcommand)]
pub(crate) enum BaselineAction {
    /// Write the current cycle hashes to a baseline file.
    Write {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// Where to write the baseline JSON (default: `[rules].baseline`
        /// under the project root).
        #[arg(value_name = "DATEI")]
        file: Option<PathBuf>,
    },
    /// Compare the current cycles against a baseline file (exit 1 on new cycles).
    Diff {
        /// Project root (default: current directory).
        #[arg(value_name = "PFAD")]
        path: Option<PathBuf>,
        /// The baseline JSON to compare against (default: `[rules].baseline`
        /// under the project root).
        #[arg(value_name = "DATEI")]
        file: Option<PathBuf>,
    },
}
