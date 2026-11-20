//! "Open in editor": runs the editor command of `desktop.toml` (`editor =
//! "code -g {file}:{line}"`) on a project file. The command is an argument
//! list, never a shell line: the template splits at spaces (double quotes
//! keep a word with spaces together), then `{file}` (absolute, inside the
//! project) and `{line}` are replaced inside each word.

use std::path::{Path, PathBuf};

/// The presets of the editor picker: label and command template.
pub const PRESETS: [(&str, &str); 3] = [
    ("VS Code", "code -g {file}:{line}"),
    ("Cursor", "cursor -g {file}:{line}"),
    ("JetBrains", "idea --line {line} {file}"),
];

/// The `desktop.toml` key.
const KEY: &str = "editor";

/// The template's words; `"…"` keeps spaces inside a word.
fn words(template: &str) -> Vec<String> {
    let (mut out, mut word, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    for c in template.chars() {
        match c {
            '"' => (quoted, any) = (!quoted, true),
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut word));
                }
                any = false;
            }
            c => {
                word.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(word);
    }
    out
}

/// The program and its arguments for `file` at `line` (line 0 opens at 1).
pub fn command(template: &str, file: &Path, line: u32) -> Vec<String> {
    let file = file.to_string_lossy();
    let line = line.max(1).to_string();
    (words(template).into_iter())
        .map(|w| w.replace("{file}", &file).replace("{line}", &line))
        .collect()
}

/// `rel` below `root` as an absolute path, if it exists and stays inside
/// the project (no `..` or link out of it).
pub fn inside(root: &Path, rel: &str) -> Option<PathBuf> {
    let root = root.canonicalize().ok()?;
    let path = root.join(rel).canonicalize().ok()?;
    path.starts_with(&root).then_some(path)
}

/// The configured template, else the first preset.
pub fn template() -> String {
    crate::settings_file::get_doc(|d| d.get(KEY)?.as_str().map(str::to_string))
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| PRESETS[0].1.to_string())
}

/// Store `template` (empty: back to the default).
pub fn set_template(template: &str) {
    crate::settings_file::update_doc(|d| match template.trim() {
        "" => {
            d.remove(KEY);
        }
        t => d[KEY] = toml_edit::value(t),
    });
}

/// The preset index of `template`; [`PRESETS`]`.len()` for a custom one.
pub fn preset_of(template: &str) -> usize {
    (PRESETS.iter())
        .position(|(_, t)| *t == template.trim())
        .unwrap_or(PRESETS.len())
}

/// Open `rel` (project-relative) at `line` with the configured editor.
pub fn open(root: &Path, rel: &str, line: u32) -> Result<(), String> {
    let file = inside(root, rel).ok_or_else(|| format!("{rel}: not a file of this project"))?;
    let args = command(&template(), &file, line);
    let (program, rest) = args.split_first().ok_or("the editor command is empty")?;
    std::process::Command::new(program)
        .args(rest)
        .spawn()
        .map(drop)
        .map_err(|e| format!("{program}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_put_file_and_line_into_their_arguments() {
        let f = Path::new("/w/shop/Orders Api.cs");
        assert_eq!(
            command(PRESETS[0].1, f, 42),
            ["code", "-g", "/w/shop/Orders Api.cs:42"]
        );
        assert_eq!(
            command(PRESETS[2].1, f, 0),
            ["idea", "--line", "1", "/w/shop/Orders Api.cs"],
            "line 0 opens at 1; the path stays one argument"
        );
    }

    #[test]
    fn a_custom_template_keeps_quoted_words_and_never_runs_a_shell() {
        let t = r#""/Applications/My Editor.app/bin/ed"  --goto {file}:{line} ; rm -rf ~"#;
        let args = command(t, Path::new("/p/$(x).rs"), 3);
        assert_eq!(
            args,
            [
                "/Applications/My Editor.app/bin/ed",
                "--goto",
                "/p/$(x).rs:3",
                ";",
                "rm",
                "-rf",
                "~"
            ],
            "every word is a plain argument of the program"
        );
        assert_eq!(preset_of(" cursor -g {file}:{line} "), 1);
        assert_eq!(preset_of(t), PRESETS.len(), "custom");
    }

    #[test]
    fn only_files_inside_the_project_resolve() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("p");
        std::fs::create_dir_all(root.join("src")).expect("dir");
        std::fs::write(root.join("src/a.rs"), "").expect("file");
        std::fs::write(dir.path().join("secret"), "").expect("file");
        let abs = inside(&root, "src/a.rs").expect("inside");
        assert!(abs.is_absolute() && abs.ends_with("src/a.rs"));
        assert_eq!(inside(&root, "../secret"), None, "outside the project");
        assert_eq!(inside(&root, "src/missing.rs"), None);
    }
}
