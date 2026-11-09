# ADR 0008: arc42 document language

- **Status:** accepted
- **Date:** 2026-11-09

## Context

`archguard-report` writes every generated arc42 text in German: chapter
titles, the generated blocks, diagram titles and notes. The desktop's
English window only says so ("Document language: German …"). The user
asked (2026-11-09) that arc42 also speaks English.

The saved `arc42.md` keeps the user's prose; regeneration only replaces
the generated blocks between `<!-- archguard:arc42:NN:start/end -->`
markers. The markers do not depend on the language.

## Decision

- `archguard_core::DocLang` (`En`, `De`) selects the language of all
  generated arc42 text. It is the run-time field `Arc42Config::lang`
  (`#[serde(skip)]`: never read from or written to `archguard.toml`) and
  travels in `Arc42Inputs`.
- The desktop passes the language of its window. CLI and MCP have no
  window and use the default.
- The default is English.
- Both languages are written next to each other in the code that renders
  the text (`DocLang::pick(en, de)` or a `match`), not through a catalogue:
  most texts are format strings with values, and the German wording of
  existing documents must stay as it is.

## Alternatives

- **A project setting** (`[arc42] language` in `archguard.toml`): every
  client writes the same language. Rejected by the user: the document
  follows the window.
- **German as default**: no change for existing documents. Rejected by the
  user: English is the default.
- **A gettext catalogue** as in the desktop: one key per sentence, but
  the format strings would need positional placeholders and the German
  text would move away from the code that builds it.

## Consequences

- Two people with different window languages write different generated
  blocks into the same `arc42.md`. The markers keep the merge correct;
  the diff shows the language switch.
- Headings and the file header already in a saved `arc42.md` belong to
  the user and keep their language; a new document gets the selected one.
- CLI and MCP output changes from German to English.
- New generated text needs both languages; tests pin one language each.

## History

- 2026-11-09: the language moved from an extra parameter of
  `arc42_document` / `arc42_from_evidence` into `Arc42Config::lang`, so
  the engine functions keep six parameters (compactness ceiling of the
  pre-commit hook) and CLI and MCP calls stay unchanged.
