//! Parsers of lockfiles (and the `.csproj` manifest) for the dependency
//! inventory and the SBOM (ADR 0016). Each returns what the file states;
//! licences and scopes the file does not state stay empty/`None`.

/// One dependency as a lockfile or manifest states it. `licenses` and
/// `scope` stay empty/`None` when the file does not say (never guessed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Dep {
    pub(crate) name: String,
    pub(crate) version: Option<String>,
    /// Licence strings as stated (SPDX ids or expressions, sometimes free text).
    pub(crate) licenses: Vec<String>,
    /// `Some("optional")` for dev/test-only, `Some("required")` where the
    /// file marks runtime use, `None` when the file does not say.
    pub(crate) scope: Option<&'static str>,
    /// A first-party package (workspace member, path/editable/project
    /// reference); counted by the inventory, left out of the SBOM.
    pub(crate) local: bool,
}

pub(crate) fn dep(name: impl Into<String>, version: Option<String>) -> Dep {
    Dep {
        name: name.into(),
        version,
        ..Dep::default()
    }
}

pub(crate) fn scope(optional: bool) -> Option<&'static str> {
    Some(if optional { "optional" } else { "required" })
}

/// `[[package]]` entries of a TOML lockfile (`Cargo.lock`, `poetry.lock`,
/// `uv.lock`). Cargo: no `source` = workspace/path crate. uv: an
/// editable/virtual/directory/path source is the project itself; Poetry:
/// a `directory`/`file` source `type`. Poetry `category = "dev"` (lock
/// format 1.x) marks dev-only packages.
fn toml_lock_deps(text: &str, eco: &str) -> Vec<Dep> {
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    let Some(packages) = table.get("package").and_then(toml::Value::as_array) else {
        return Vec::new();
    };
    packages
        .iter()
        .filter_map(|package| {
            let name = package.get("name")?.as_str()?;
            let version = package
                .get("version")
                .and_then(toml::Value::as_str)
                .map(str::to_string);
            let source = package.get("source");
            let local = match eco {
                "cargo" => source.is_none(),
                _ => source.and_then(toml::Value::as_table).is_some_and(|s| {
                    ["editable", "virtual", "directory", "path"]
                        .iter()
                        .any(|k| s.contains_key(*k))
                        || matches!(
                            s.get("type").and_then(toml::Value::as_str),
                            Some("directory" | "file")
                        )
                }),
            };
            let dev = package.get("category").and_then(toml::Value::as_str) == Some("dev");
            Some(Dep {
                local,
                scope: dev.then_some("optional"),
                ..dep(name, version)
            })
        })
        .collect()
}

/// `package-lock.json` (v2/v3 `packages` map preferred, v1
/// `dependencies` as fallback). The root package entry (`""`) is not a
/// dependency of the project; `link: true` entries are workspace packages.
/// An alias (`node_modules/<alias>` with `name`) is the named package.
fn package_lock_deps(text: &str) -> Vec<Dep> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let entry_dep = |name: &str, entry: &serde_json::Value| {
        let flag = |key: &str| entry.get(key).and_then(serde_json::Value::as_bool) == Some(true);
        Dep {
            licenses: entry
                .get("license")
                .and_then(serde_json::Value::as_str)
                .map(|l| vec![l.to_string()])
                .unwrap_or_default(),
            scope: scope(flag("dev") || flag("devOptional")),
            local: flag("link"),
            ..dep(
                name,
                entry
                    .get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            )
        }
    };
    let mut deps = Vec::new();
    if let Some(packages) = root.get("packages").and_then(serde_json::Value::as_object) {
        for (key, entry) in packages {
            // v3 keys are paths; only `…/node_modules/<name>` entries are
            // third-party — the root `""` and workspace entries are us.
            let Some((_, tail)) = key.rsplit_once("node_modules/") else {
                continue;
            };
            if !tail.is_empty() {
                let name = entry.get("name").and_then(serde_json::Value::as_str);
                deps.push(entry_dep(name.unwrap_or(tail), entry));
            }
        }
    } else if let Some(dependencies) = root
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
    {
        for (name, entry) in dependencies {
            deps.push(entry_dep(name, entry));
        }
    }
    deps
}

/// Split an npm spec like `@scope/pkg@^1.0` or `pkg@npm:^1.0` into its
/// name (the `@` at index 0 belongs to the scope). An alias
/// `alias@npm:real@^1.0` installs `real`, so that is the name.
fn npm_spec_name(spec: &str) -> &str {
    let spec = spec.trim().trim_matches('"').trim_matches('\'');
    let skip = usize::from(spec.starts_with('@'));
    let Some(at) = spec[skip..].find('@').map(|at| at + skip) else {
        return spec;
    };
    match spec[at + 1..].strip_prefix("npm:") {
        Some(real) if npm_spec_name(real).len() < real.len() => npm_spec_name(real),
        _ => &spec[..at],
    }
}

/// `yarn.lock` v1 and berry: an unindented header lists the specs, the
/// indented `version` line holds the resolved version. Berry
/// `@workspace:`/`@link:`/`@portal:` entries are the project itself.
fn yarn_lock_deps(text: &str) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut current: Option<Dep> = None;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            deps.extend(current.take());
            let header = line.trim_end_matches(':');
            let first = header.split(", ").next().unwrap_or_default();
            if first.contains("__metadata") {
                continue;
            }
            let local = ["@workspace:", "@link:", "@portal:", "@file:"]
                .iter()
                .any(|p| first.contains(p));
            current = Some(Dep {
                local,
                ..dep(npm_spec_name(first), None)
            });
        } else if let Some(entry) = current.as_mut()
            && let Some(rest) = line
                .strip_prefix("  version")
                .filter(|rest| rest.starts_with([' ', ':']))
        {
            let version = rest.trim_start_matches(':').trim().trim_matches('"');
            entry.version = (!version.is_empty()).then(|| version.to_string());
        }
    }
    deps.extend(current);
    deps
}

/// `pnpm-lock.yaml` `packages` keys: v5 `/name/1.0.0`, v6 `/name@1.0.0`,
/// v9 `name@1.0.0`; peer suffixes `(…)`/`_…` are dropped.
fn pnpm_lock_deps(text: &str) -> Vec<Dep> {
    let Ok(root) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(text) else {
        return Vec::new();
    };
    let Some(packages) = root.get("packages").and_then(|p| p.as_mapping()) else {
        return Vec::new();
    };
    let v5 = root
        .get("lockfileVersion")
        // `5.4-inlineSpecifiers`: the leading number is the version.
        .and_then(|v| {
            v.as_f64()
                .or_else(|| v.as_str()?.split('-').next()?.parse().ok())
        })
        .is_some_and(|v| v < 6.0);
    packages
        .iter()
        .filter_map(|(key, entry)| {
            let key = key.as_str()?.trim_start_matches('/');
            let key = key.split('(').next()?;
            let (name, version) = if v5 {
                let (name, version) = key.rsplit_once('/')?;
                (name, version.split('_').next()?)
            } else {
                let name = npm_spec_name(key);
                (name, key.get(name.len() + 1..)?)
            };
            let dev = entry.get("dev").and_then(|d| d.as_bool());
            Some(Dep {
                scope: dev.and_then(scope),
                ..dep(name, Some(version.to_string()))
            })
        })
        .collect()
}

/// NuGet `packages.lock.json`: resolved packages of every target
/// framework; `Project` entries are references to own projects.
fn nuget_lock_deps(text: &str) -> Vec<Dep> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for framework in root
        .get("dependencies")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|f| f.values())
        .filter_map(serde_json::Value::as_object)
    {
        for (name, entry) in framework {
            let kind = entry.get("type").and_then(serde_json::Value::as_str);
            let version = entry
                .get("resolved")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            deps.push(Dep {
                local: kind == Some("Project"),
                ..dep(name, version)
            });
        }
    }
    deps
}

/// `*.csproj` `<PackageReference Include=… Version=…>` (attribute or child
/// element). Centrally managed versions stay unknown.
pub(crate) fn csproj_deps(text: &str) -> Vec<Dep> {
    let Ok(document) = roxmltree::Document::parse(text) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|n| n.has_tag_name("PackageReference"))
        .filter_map(|n| {
            let name = n.attribute("Include")?;
            let version = n
                .attribute("Version")
                .or_else(|| {
                    n.children()
                        .find(|c| c.has_tag_name("Version"))
                        .and_then(|c| c.text())
                })
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string);
            Some(dep(name, version))
        })
        .collect()
}

/// `gradle.lockfile`: `group:artifact:version=classpath,…`; a package
/// only on test classpaths is optional.
fn gradle_lock_deps(text: &str) -> Vec<Dep> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(|line| {
            let (coordinate, classpaths) = line.split_once('=')?;
            let mut parts = coordinate.split(':');
            let (group, artifact, version) = (parts.next()?, parts.next()?, parts.next()?);
            let test_only = classpaths.split(',').all(|c| c.starts_with("test"));
            Some(Dep {
                scope: scope(test_only),
                ..dep(format!("{group}:{artifact}"), Some(version.to_string()))
            })
        })
        .collect()
}

/// `go.mod` `require` lines (single and block form). Go ≥1.17 lists the
/// whole build list here, so `go.sum` (which also has unused versions) is
/// not read.
fn go_mod_deps(text: &str) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        let spec = if in_block {
            if line == ")" {
                in_block = false;
                continue;
            }
            line
        } else if line == "require (" {
            in_block = true;
            continue;
        } else if let Some(rest) = line.strip_prefix("require ") {
            rest
        } else {
            continue;
        };
        let mut parts = spec.split_whitespace();
        if let (Some(module), Some(version)) = (parts.next(), parts.next()) {
            deps.push(dep(module, Some(version.to_string())));
        }
    }
    deps
}

/// `composer.lock`: `packages` (required) and `packages-dev` (optional)
/// with their stated `license` arrays.
fn composer_lock_deps(text: &str) -> Vec<Dep> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    for (key, optional) in [("packages", false), ("packages-dev", true)] {
        for package in root
            .get(key)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = package.get("name").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let field = |k: &str| package.get(k).and_then(serde_json::Value::as_str);
            deps.push(Dep {
                licenses: package
                    .get("license")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect(),
                scope: scope(optional),
                local: package
                    .pointer("/dist/type")
                    .and_then(serde_json::Value::as_str)
                    == Some("path"),
                ..dep(name, field("version").map(str::to_string))
            });
        }
    }
    deps
}

/// Dependencies of a lockfile by its file name; `None` for other files.
pub(crate) fn lockfile_deps(name: &str, text: &str) -> Option<Vec<Dep>> {
    Some(match name {
        "Cargo.lock" => toml_lock_deps(text, "cargo"),
        "poetry.lock" | "uv.lock" => toml_lock_deps(text, "pypi"),
        "package-lock.json" => package_lock_deps(text),
        "yarn.lock" => yarn_lock_deps(text),
        "pnpm-lock.yaml" => pnpm_lock_deps(text),
        "composer.lock" => composer_lock_deps(text),
        "gradle.lockfile" => gradle_lock_deps(text),
        "packages.lock.json" => nuget_lock_deps(text),
        "go.mod" => go_mod_deps(text),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(deps: &[Dep]) -> Vec<(&str, Option<&str>, bool)> {
        deps.iter()
            .map(|d| (d.name.as_str(), d.version.as_deref(), d.local))
            .collect()
    }

    #[test]
    fn a_leading_multibyte_char_does_not_panic() {
        assert_eq!(npm_spec_name("\u{FEFF}x@1"), "\u{FEFF}x");
        assert_eq!(npm_spec_name("é"), "é");
    }

    #[test]
    fn npm_aliases_name_the_installed_package() {
        let lock = package_lock_deps(
            r#"{"packages":{"node_modules/string-width-cjs":{"name":"string-width","version":"4.2.3"}}}"#,
        );
        assert_eq!(names(&lock), [("string-width", Some("4.2.3"), false)]);
        let yarn = yarn_lock_deps(
            "\"string-width-cjs@npm:string-width@^4.2.0\":\n  version \"4.2.3\"\n\n\"@a/b-alias@npm:@a/b@^1\":\n  version \"1.0.0\"\n",
        );
        assert_eq!(
            names(&yarn),
            [
                ("string-width", Some("4.2.3"), false),
                ("@a/b", Some("1.0.0"), false)
            ]
        );
    }

    #[test]
    fn poetry_directory_and_file_sources_are_first_party() {
        let deps = toml_lock_deps(
            "[[package]]\nname = \"mylib\"\nversion = \"0.1.0\"\n[package.source]\ntype = \"directory\"\nurl = \"../mylib\"\n\n[[package]]\nname = \"wheel-dep\"\nversion = \"1.0\"\n[package.source]\ntype = \"file\"\nurl = \"dist/w.whl\"\n\n[[package]]\nname = \"gitdep\"\nversion = \"2.0\"\n[package.source]\ntype = \"git\"\nurl = \"https://example.org/g.git\"\n",
            "pypi",
        );
        assert_eq!(
            names(&deps),
            [
                ("mylib", Some("0.1.0"), true),
                ("wheel-dep", Some("1.0"), true),
                ("gitdep", Some("2.0"), false)
            ]
        );
    }

    #[test]
    fn pnpm_inline_specifiers_version_reads_as_v5() {
        let deps = pnpm_lock_deps(
            "lockfileVersion: 5.4-inlineSpecifiers\npackages:\n  /left-pad/1.3.0:\n    dev: false\n",
        );
        assert_eq!(names(&deps), [("left-pad", Some("1.3.0"), false)]);
    }

    #[test]
    fn yarn_berry_and_older_pnpm_keys_are_read() {
        let berry = yarn_lock_deps(
            "__metadata:\n  version: 8\n\n\"@scope/a@npm:^1.0.0, @scope/a@npm:^1.1.0\":\n  version: 1.2.0\n  dependencies:\n    version-x: ^1\n\n\"app@workspace:.\":\n  version: 0.0.0-use.local\n",
        );
        let berry: Vec<(&str, Option<&str>, bool)> = berry
            .iter()
            .map(|d| (d.name.as_str(), d.version.as_deref(), d.local))
            .collect();
        assert_eq!(
            berry,
            [
                ("@scope/a", Some("1.2.0"), false),
                ("app", Some("0.0.0-use.local"), true)
            ]
        );
        let v5 = pnpm_lock_deps(
            "lockfileVersion: 5.4\npackages:\n  /@scope/a/1.0.0_react@18.0.0:\n    dev: true\n",
        );
        assert_eq!(
            (v5[0].name.as_str(), v5[0].version.as_deref(), v5[0].scope),
            ("@scope/a", Some("1.0.0"), Some("optional"))
        );
        let v6 = pnpm_lock_deps(
            "lockfileVersion: '6.0'\npackages:\n  /left-pad@1.3.0:\n    dev: false\n",
        );
        assert_eq!(
            (v6[0].name.as_str(), v6[0].version.as_deref(), v6[0].scope),
            ("left-pad", Some("1.3.0"), Some("required"))
        );
    }
}
