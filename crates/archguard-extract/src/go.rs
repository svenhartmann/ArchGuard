//! Go import declarations. Package dependencies stay package-level; imports do
//! not imply that every file in the target package is used. No SDK is needed.
use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::visibility::Visibility;
use crate::walk::Lang;

pub struct GoExtractor;
#[derive(Clone)]
struct Token {
    text: String,
    line: u32,
    literal: bool,
}
// Go interpreted strings also allow byte/octal escapes, unlike JSON strings.
fn interpreted_string(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut chars = text.strip_prefix('"')?.strip_suffix('"')?.chars();
    let mut output = Vec::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            if matches!(ch, '\n' | '\r' | '"') {
                return None;
            }
            let mut buf = [0; 4];
            output.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        let escape = chars.next()?;
        let decoded = match escape {
            'a' => 7,
            'b' => 8,
            'f' => 12,
            'n' => 10,
            'r' => 13,
            't' => 9,
            'v' => 11,
            '\\' => b'\\',
            '"' => b'"',
            'x' | 'u' | 'U' => {
                let length = match escape {
                    'x' => 2,
                    'u' => 4,
                    _ => 8,
                };
                let digits: String = chars.by_ref().take(length).collect();
                if digits.len() != length || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return None;
                }
                let number = u32::from_str_radix(&digits, 16).ok()?;
                if escape == 'x' {
                    u8::try_from(number).ok()?
                } else {
                    let value = char::from_u32(number)?;
                    let mut buf = [0; 4];
                    output.extend_from_slice(value.encode_utf8(&mut buf).as_bytes());
                    continue;
                }
            }
            '0'..='7' => {
                let mut digits = String::from(escape);
                digits.extend(chars.by_ref().take(2));
                if digits.len() != 3 {
                    return None;
                }
                u8::from_str_radix(&digits, 8).ok()?
            }
            _ => return None,
        };
        output.push(decoded);
    }
    String::from_utf8(output).ok()
}
fn tokens(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut line = 1;
    let mut result = vec![];
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            if bytes[i] == b'\n' {
                line += 1;
            }
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            i += 2;
            while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        let start = i;
        let at = line;
        if matches!(bytes[i], b'"' | b'\'' | b'`') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\n' {
                    line += 1;
                }
                if quote != b'`' && bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == quote {
                    break;
                }
                i += 1;
            }
            let closed = i < bytes.len();
            i = (i + 1).min(bytes.len());
            let value = if !closed {
                None
            } else if quote == b'`' {
                Some(String::from_utf8_lossy(&bytes[start + 1..i - 1]).replace('\r', ""))
            } else if quote == b'"' {
                interpreted_string(&bytes[start..i])
            } else {
                None
            };
            result.push(Token {
                text: value.unwrap_or_default(),
                line: at,
                literal: true,
            });
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' || bytes[i] >= 128 {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] >= 128)
            {
                i += 1;
            }
        } else {
            i += 1;
        }
        result.push(Token {
            text: String::from_utf8_lossy(&bytes[start..i]).into_owned(),
            line: at,
            literal: false,
        });
    }
    result
}
fn import_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '\\' | '"' | '\'' | '`'))
        && !value
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
}
/// Literal module identity only: replace directives and toolchain commands are
/// not executed. The engine uses the nearest manifest for each selected file.
pub fn module_path(source: &str) -> Option<String> {
    let declarations: Vec<_> = source
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("module ")
                .or_else(|| line.trim().strip_prefix("module\t"))
        })
        .collect();
    let [declaration] = declarations.as_slice() else {
        return None;
    };
    let value = declaration.split("//").next()?.trim();
    let value = if value.starts_with('"') {
        interpreted_string(value.as_bytes())?
    } else {
        value.to_owned()
    };
    import_path(&value).then_some(value)
}
impl Extractor for GoExtractor {
    fn lang(&self) -> Lang {
        Lang::Go
    }
    fn extract(&self, rel_path: &str, source: &str) -> Extraction {
        let ts = tokens(source);
        let lines: Vec<_> = source.lines().collect();
        let mut result = Extraction::default();
        let mut depth = 0;
        let mut at = 0;
        while at < ts.len() {
            let t = &ts[at];
            if !t.literal && t.text == "{" {
                depth += 1;
            }
            if !t.literal && t.text == "}" {
                depth -= 1;
            }
            if depth == 0
                && !t.literal
                && t.text == "package"
                && ts.get(at + 1).is_some_and(|t| !t.literal)
            {
                result.declarations.push(Declaration {
                    module_path: rel_path.rsplit_once('/').map_or("", |(dir, _)| dir).into(),
                    file: rel_path.into(),
                });
            }
            if depth == 0 && !t.literal && t.text == "import" {
                at += 1;
                let group = ts.get(at).is_some_and(|t| !t.literal && t.text == "(");
                if group {
                    at += 1;
                }
                while at < ts.len() {
                    let t = &ts[at];
                    if group && !t.literal && t.text == ")" {
                        break;
                    }
                    if t.literal {
                        if import_path(&t.text) {
                            result.imports.push(RawImport {
                                source: rel_path.into(),
                                target: TargetSpec::Symbol {
                                    path: t.text.clone(),
                                    stem: None,
                                },
                                line: t.line,
                                raw: lines
                                    .get(t.line.saturating_sub(1) as usize)
                                    .unwrap_or(&"")
                                    .to_string(),
                            });
                        }
                        if !group {
                            break;
                        }
                    } else if !group && t.line > ts[at.saturating_sub(1)].line {
                        break;
                    }
                    at += 1;
                }
            }
            at += 1;
        }
        result
    }
}
pub fn scan_visibility(source: &str) -> Visibility {
    let ts = tokens(source);
    let mut result = Visibility::default();
    let mut depth = 0;
    for (i, t) in ts.iter().enumerate() {
        if !t.literal && t.text == "{" {
            depth += 1;
        }
        if !t.literal && t.text == "}" {
            depth -= 1;
        }
        if depth != 0 || t.literal || !matches!(t.text.as_str(), "func" | "type" | "var" | "const")
        {
            continue;
        }
        let Some(name) = ts.get(i + 1).filter(|t| {
            !t.literal
                && t.text
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
        }) else {
            continue;
        };
        if name.text.chars().next().is_some_and(char::is_uppercase) {
            result.exported.insert(name.text.clone());
        } else {
            result.internal.insert(name.text.clone());
        }
    }
    result
}
