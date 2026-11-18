//! Project structure of Gradle Kotlin DSL scripts (`settings.gradle.kts`,
//! `build.gradle.kts`) as written: `include("…")` and `project("…")` calls
//! with plain string literals. Nothing is evaluated, so any other argument
//! is reported as computed. Comments and strings never create a call.

use super::tokens::{Tok, merged, pairs};

/// The `include` and `project` calls of one script.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GradleScript {
    /// One entry per `include(…)` argument.
    pub includes: Vec<GradleRef>,
    /// One entry per `project(…)` call.
    pub projects: Vec<GradleRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradleRef {
    /// The project path as written (`:a:b`), `None` when it is computed.
    pub path: Option<String>,
    pub line: u32,
    /// The call `project(…)` is the only argument of, such as
    /// `implementation`.
    pub configuration: Option<String>,
}

/// The `include` and `project` calls of `source`; `None` when its brackets
/// are unbalanced.
pub fn scan_gradle_script(source: &str) -> Option<GradleScript> {
    let toks = merged(source);
    let pairs = pairs(&toks)?;
    let text = |i: usize| toks.get(i).filter(|t| !t.literal).map_or("", |t| &t.text);
    let mut script = GradleScript::default();
    for (i, tok) in toks.iter().enumerate() {
        let call = text(i);
        if !matches!(call, "include" | "project")
            || text(i + 1) != "("
            || (i > 0 && matches!(text(i - 1), "." | "?." | "::" | "fun"))
        {
            continue;
        }
        let close = pairs[&(i + 1)];
        let args = &toks[i + 2..close];
        if call == "include" {
            // `include(":a", ":b")`: literals between commas.
            for arg in args.split(|t| !t.literal && t.text == ",") {
                script.includes.push(GradleRef {
                    path: plain(arg),
                    line: arg.first().map_or(tok.line, |t| t.line),
                    configuration: None,
                });
            }
        } else {
            let wrapped = i >= 2 && text(i - 1) == "(" && pairs[&(i - 1)] == close + 1;
            script.projects.push(GradleRef {
                path: plain(args),
                line: tok.line,
                configuration: (wrapped && toks[i - 2].is_name()).then(|| text(i - 2).into()),
            });
        }
    }
    Some(script)
}

/// The content of a single plain string literal without templates or
/// escapes.
fn plain(arg: &[Tok]) -> Option<String> {
    let [tok] = arg else { return None };
    let inner = tok.text.strip_prefix('"')?.strip_suffix('"')?;
    (tok.literal && !inner.is_empty() && !inner.contains(['"', '$', '\\'])).then(|| inner.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(refs: &[GradleRef]) -> Vec<(Option<&str>, Option<&str>)> {
        refs.iter()
            .map(|r| (r.path.as_deref(), r.configuration.as_deref()))
            .collect()
    }

    #[test]
    fn literal_includes_and_project_dependencies_are_read() {
        let script = scan_gradle_script(
            "rootProject.name = \"shop\"\n\
             include(\":app\", \":libs:util\")\n\
             // include(\":ghost\")\n\
             val text = \"\"\"include(\":raw\")\"\"\"\n\
             include(\":$name\")\n\
             dependencies {\n\
                 implementation(project(\":core\"))\n\
                 testImplementation(project(\":libs:util\"))\n\
                 implementation(project(path))\n\
                 evaluationDependsOn(project(\":core\").path)\n\
             }\n",
        )
        .unwrap();
        assert_eq!(
            paths(&script.includes),
            [
                (Some(":app"), None),
                (Some(":libs:util"), None),
                (None, None)
            ]
        );
        assert_eq!(
            paths(&script.projects),
            [
                (Some(":core"), Some("implementation")),
                (Some(":libs:util"), Some("testImplementation")),
                (None, Some("implementation")),
                // Not the only argument: no dependency declaration.
                (Some(":core"), None),
            ]
        );
        assert_eq!(script.projects[0].line, 7);
    }

    #[test]
    fn unbalanced_scripts_are_not_read() {
        assert_eq!(scan_gradle_script("include(\":a\""), None);
    }
}
