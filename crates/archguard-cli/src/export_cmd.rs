//! Exports for other tools (ADR 0013): the Structurizr DSL of
//! `graph --format structurizr` and the arc42 document as AsciiDoc or
//! Word (`doc --arc42 --format asciidoc|docx`).
use crate::args::{Cli, Command, Format};
use crate::output::{EXIT_FOUND, EXIT_OK, analysis_error, emit, usage};
use archguard_engine::LoadedConfig;
use archguard_engine::core::{Graph, Report};
use archguard_engine::report::{markdown_to_asciidoc, markdown_to_docx};
use std::path::{Path, PathBuf};

/// The exports that run outside the shared analysis: usage errors for an
/// export format outside its command (`structurizr` belongs to `graph`,
/// `asciidoc` and `docx` to `doc --arc42`), and `graph --format structurizr`.
pub(crate) fn standalone(cli: &Cli, root: &Path) -> Option<i32> {
    let home = match (&cli.command, cli.format) {
        (Command::Graph { format, .. }, _) if *format == Format::Structurizr => {
            return Some(graph(cli, root));
        }
        (
            Command::Graph {
                service: Some(_), ..
            },
            _,
        ) => return Some(usage("--service applies to --format structurizr only")),
        (command, Format::Structurizr) => matches!(command, Command::Graph { .. }),
        (command, Format::Asciidoc | Format::Docx) => matches!(command, Command::Doc { .. }),
        _ => return None,
    };
    (!home).then(|| {
        usage(format!(
            "invalid value '{}' for '--format': structurizr is a graph format, asciidoc and docx are doc --arc42 formats",
            cli.format_name()
        ))
    })
}

/// `graph --format structurizr`: the whole topology, no graph cut.
fn graph(cli: &Cli, root: &Path) -> i32 {
    let Command::Graph {
        focus,
        depth,
        expand,
        service,
        ..
    } = &cli.command
    else {
        unreachable!("dispatched for graph only")
    };
    if focus.is_some() || depth.is_some() || !expand.is_empty() {
        return usage(
            "structurizr exports the whole service topology; --focus, --depth and --expand do not apply",
        );
    }
    let analysed = archguard_engine::load_config(cli.config.as_deref(), root)
        .and_then(|loaded| archguard_engine::analyze_run(root, &loaded, cli.command.level()));
    match analysed {
        Ok(run) => structurizr(
            cli,
            root,
            service.as_deref(),
            &run.effective_config(),
            &run.graph,
        ),
        Err(e) => analysis_error(e),
    }
}

/// `doc`: the arc42 exports here, Markdown in `insights_cmd::doc`.
pub(crate) fn doc(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
) -> i32 {
    let Command::Doc {
        out,
        check,
        arc42,
        chapter,
        stdout,
        ..
    } = &cli.command
    else {
        unreachable!("dispatched for doc only")
    };
    if !matches!(cli.format, Format::Asciidoc | Format::Docx) {
        let options = crate::insights_cmd::DocOptions {
            out: out.as_deref(),
            check: *check,
            arc42: *arc42,
            chapter: *chapter,
            stdout: *stdout,
        };
        return crate::insights_cmd::doc(cli, root, loaded, graph, report, options);
    }
    if !*arc42 {
        return usage("asciidoc and docx export the arc42 document; add --arc42");
    }
    let options = Arc42Export {
        out: out.as_deref(),
        chapter: *chapter,
        stdout: *stdout,
        check: *check,
    };
    arc42_export(cli, root, loaded, graph, report, options)
}

/// `graph --format structurizr [--service ID]`, saved as
/// `.archguard/reports/graph.dsl` like the other graph formats.
fn structurizr(
    cli: &Cli,
    root: &Path,
    service: Option<&str>,
    loaded: &LoadedConfig,
    graph: &Graph,
) -> i32 {
    let topology = match archguard_engine::analyze_services(root) {
        Ok(t) => t,
        Err(e) => return analysis_error(e),
    };
    let selected = match service.map(|s| archguard_engine::c4_service(&topology, s)) {
        Some(Err(message)) => return usage(message),
        Some(Ok(s)) => Some(s),
        None => None,
    };
    let dsl = match archguard_engine::structurizr_workspace(
        root,
        &loaded.config,
        graph,
        &topology,
        selected,
    ) {
        Ok(dsl) => dsl,
        Err(e) => return usage(e.to_string()),
    };
    if let Err(e) = crate::project_files::write_report(root, "graph.dsl", &dsl) {
        return analysis_error(e);
    }
    if !cli.quiet {
        emit(&dsl);
    }
    EXIT_OK
}

struct Arc42Export<'a> {
    out: Option<&'a Path>,
    chapter: Option<u8>,
    stdout: bool,
    check: bool,
}

/// `doc --arc42 --format asciidoc|docx`: the arc42 Markdown, with the
/// own text of `<root>/arc42.md` when it exists (one chapter: generated
/// only), converted; `arc42.md` itself is only read.
fn arc42_export(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
    options: Arc42Export<'_>,
) -> i32 {
    let docx = cli.format == Format::Docx;
    if docx && options.stdout {
        return usage("docx is a binary file; write it with --out instead of --stdout");
    }
    let markdown = match markdown(root, loaded, graph, report, options.chapter) {
        Ok(m) => m,
        Err(code) => return code,
    };
    let bytes = if docx {
        markdown_to_docx(&markdown)
    } else {
        markdown_to_asciidoc(&markdown).into_bytes()
    };
    if options.stdout {
        if !cli.quiet {
            emit(&String::from_utf8_lossy(&bytes));
        }
        return EXIT_OK;
    }
    let target = match target(root, &options, docx) {
        Ok(t) => t,
        Err(code) => return code,
    };
    if options.check {
        return check(cli, &target, &bytes);
    }
    if let Err(code) = create_parent(root, &target) {
        return code;
    }
    match write_file(&target, &bytes) {
        Ok(()) => {
            if !cli.quiet {
                println!("arc42 export written: {}", target.display());
            }
            EXIT_OK
        }
        Err(e) => analysis_error(e),
    }
}

/// The Markdown to convert; `Err` is the exit code.
fn markdown(
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
    chapter: Option<u8>,
) -> Result<String, i32> {
    let document = archguard_engine::arc42_document(root, &loaded.config, graph, report)
        .map_err(analysis_error)?;
    match (chapter, read_regular(&root.join("arc42.md"))) {
        (None, Ok(Some(saved))) => document.merge(&saved, None).map_err(usage),
        (_, Err(e)) => Err(analysis_error(e)),
        (chapter, _) => Ok(document.markdown(chapter)),
    }
}

/// `--check`: exit 0 if `target` holds exactly `bytes`, else 1.
fn check(cli: &Cli, target: &Path, bytes: &[u8]) -> i32 {
    let current = std::fs::read(target).is_ok_and(|saved| saved == bytes);
    if !cli.quiet {
        let state = if current {
            "is current"
        } else {
            "is missing or outdated"
        };
        println!("arc42 export {state}: {}", target.display());
    }
    if current { EXIT_OK } else { EXIT_FOUND }
}

/// The file to write: `--out`, else the name the desktop uses
/// (`export::arc42_file_name`); `Err` (exit code) for `arc42.md` itself,
/// which the export only reads.
fn target(root: &Path, options: &Arc42Export<'_>, docx: bool) -> Result<PathBuf, i32> {
    let ext = if docx { "docx" } else { "adoc" };
    let name = match options.chapter {
        Some(n) => format!("arc42-{n:02}.{ext}"),
        None => format!("arc42.{ext}"),
    };
    let target = root.join(options.out.unwrap_or(Path::new(&name)));
    if same_file(&target, &root.join("arc42.md")) {
        return Err(usage(OVERWRITES_ARC42));
    }
    Ok(target)
}

const OVERWRITES_ARC42: &str =
    "the export would overwrite arc42.md, which it only reads; choose another --out";

/// Creates the folder of `target`, then refuses `arc42.md` once more: a
/// `..` after a new folder (`new/../arc42.md`) resolves only now. Folders
/// created for a refused target are removed again.
fn create_parent(root: &Path, target: &Path) -> Result<(), i32> {
    let parent = target.parent().unwrap_or(root);
    let created: Vec<&Path> = parent.ancestors().take_while(|p| !p.exists()).collect();
    std::fs::create_dir_all(parent).map_err(analysis_error)?;
    if same_file(target, &root.join("arc42.md")) {
        for dir in created {
            // Only empty folders go; `new/..` itself is never removed.
            let _ = std::fs::remove_dir(dir);
        }
        return Err(usage(OVERWRITES_ARC42));
    }
    Ok(())
}

/// Whether `a` and `b` are the same existing file, also through another
/// spelling on a case-insensitive file system or a hard link.
fn same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        matches!((std::fs::metadata(a), std::fs::metadata(b)),
            (Ok(x), Ok(y)) if (x.dev(), x.ino()) == (y.dev(), y.ino()))
    }
    #[cfg(not(unix))]
    {
        matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
    }
}

/// The text of a regular file, `None` if it does not exist; a symlink or
/// other entry is refused, as for `doc --arc42`.
fn read_regular(path: &Path) -> std::io::Result<Option<String>> {
    use std::io::{Error, ErrorKind};
    match std::fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{} must be a regular file, not a symlink", path.display()),
        )),
        Ok(_) => std::fs::read_to_string(path).map(Some),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Replace `target` (its folder exists) as a whole through a new temporary
/// file; a final symlink or other non-regular entry is refused, as `doc` does.
fn write_file(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind, Write};
    match std::fs::symlink_metadata(target) {
        Ok(m) if !m.is_file() => {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("{} must be a regular file, not a symlink", target.display()),
            ));
        }
        Err(e) if e.kind() != ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let temp = target.with_file_name(format!(".archguard-export-{}.tmp", std::process::id()));
    let result = (|| {
        // create_new: the temporary name can never be a planted symlink.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, target)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
