# ADR 0020: Scala support with ArchGuard's token scanner

- **Status:** accepted
- **Date:** 2026-11-18

## Context

Scala projects (sbt, Scala 2 and Scala 3) were invisible: `.scala` files were
not walked. Kotlin showed the path for a JVM language without a compiler
backend: an own token scanner for imports, structure and metrics, and a
static call resolver in the engine. Scala adds two difficulties: Scala 3
indentation syntax (bodies without braces, `end` markers) and a richer
import syntax (selectors, renames, hiding, wildcards, given imports).

## Decision

1. **Files.** `.scala` is `Lang::Scala` (pipeline name `scala`). `.sc`
   worksheets and Ammonite scripts are not scanned, like Gradle `.kts`.
   `build.sbt` counts as a project manifest (ADR 0010).
2. **Lexing.** Scala reuses the Kotlin lexer flavor: nested block comments,
   `"…"` with `${…}` templates and raw `"""…"""`. An interpolator (`s"…"`)
   is merged into its string, so a string with its templates is one
   literal. Scala 2 symbol literals (`'sym`) and Scala 3 macro quotes
   (`'{…}`) are read as char literals (`shortcut:` in `lex.rs`).
3. **Import graph.** All `package` clauses of a file form one module:
   `package a.b` + `package c` and `package a { package b { … } }` both
   declare `a/b/c` or `a/b`; `package object` adds nothing. Sibling package
   blocks in one file are not told apart. An import ends at a line break
   outside braces; `{C, D => E}`, `C as D`, `_`, `*`, `given` and hiding
   `C => _` are read; `import a.B, c.D` imports twice. `scala.*`, `java.*`
   and `javax.*` are ignored. Qualified names in code count as for Java.
   Scala is in the Java import family, so Scala, Java and Kotlin resolve
   each other through the existing package + type-file rule; a nested type
   resolves to the file of its outer class; a top-level def to the package
   pseudo-node. Scala 2 relative imports (`import sibling.X` inside
   `package a`) and imports of values stay unresolved or external.
4. **Declarations.** `class`, `case class`, `trait`, `object`,
   `case object`, `enum` and given instances (`given name: T with …`, an
   anonymous one as `given_T`). Traits and `abstract` classes are abstract;
   `sealed` alone is not. Supertypes are the names after `extends`, `with`
   and `,`. A class and its companion object share one id; object members
   are static. Top-level defs belong to the file's `(module)`; defs in an
   `extension` block belong to the enclosing scope.
5. **Bodies.** A body ends at its closing brace or, without braces, at the
   first line indented no deeper than the declaration; brackets are skipped
   whole, so both syntaxes end in the same place. A def is recorded when it
   has a body (`= …`, or Scala 2 procedure syntax `{ … }`); abstract defs,
   vals, constructors and template statements are not measured.
6. **Metrics per def** (`ast-decisions-v1`, `source-maintainability/v1`):
   - cyclomatic: `if`, `while`, `for`, each `case` clause except a bare
     `case _ =>` (in `match`, `catch` and partial functions alike), `&&`,
     `||`. `match`, `catch`, `try` and `do` add nothing themselves, so a
     catch with two cases counts like Kotlin's two catch clauses. Guards
     (`case x if …`, `for (… if …)`) count as `if`.
   - cognitive: `if`, `while`, `for`, `match`, `catch` add 1 plus nesting;
     `else if` and `else` add 1; logical operator runs as in Kotlin.
   - nesting is read from lines, because indentation syntax has no braces
     to count: a line with `if`, `else`, `while`, `for`, `match`, `catch`
     or a lambda `=>` nests the deeper indented lines below it. Constructs
     on one line share that line's nesting. Brace code gives the same
     numbers as Kotlin for the shared test shapes.
   - Halstead as in Kotlin, with Scala keywords.
7. **Calls.** Only calls with an argument list are sites: `f(…)`,
   `a.b.O.m(…)`, `C[T](…)`, `new C(…)`, `new C`, `this.m(…)`,
   `super.m(…)`, `super[T].m(…)`. Parameterless calls, infix and operator
   calls and block arguments without parentheses (`Future { … }`) cannot be
   told from field reads and are not counted; extractor patterns and
   annotations are no calls. The engine resolves statically: members of the
   enclosing classes and their project supertypes, then top-level defs
   (package, imports), then types (nested, explicit import, same package,
   a wildcard with a single candidate). `O(…)` on an object with an own
   `apply` binds that `apply`; otherwise `C(…)` and `new C(…)` bind the
   constructor of a concrete class. Overloads are not told apart: only a
   unique def resolves. A name imported explicitly from a package outside
   the project is `external`; everything else that is not found stays
   `unresolved`, including implicits and givens, extension methods, calls
   on values and call results, every call inside an `extension` block and
   calls into Java or Kotlin classes of the project.

## Alternatives

- **scalac / Scala 3 compiler or Metals (SemanticDB):** exact, but needs a
  JVM, sbt and a resolved classpath, and runs build code. Rejected for the
  same reasons as for Kotlin; it stays an option for a later opt-in.
- **A tree-sitter grammar:** a new native dependency for one language and
  still no name resolution. Rejected (YAGNI).
- **Brace-based nesting as in Kotlin:** gives no nesting at all for
  Scala 3 indentation syntax. Rejected in favour of line-based nesting.

## Consequences

- Scala projects get an import graph, class metrics and partial call
  coverage without any installation; the report is `partial`, as for
  Kotlin and PHP.
- Line-based nesting differs from brace counting for constructs that share
  one line (`xs.map(x => if …)` is not nested).
- A `;` ends a brace-less body, so several statements on one line of an
  indentation body cut that def short. Givens bodies are measured; given
  and implicit resolution is not attempted.
- Merging a class with its companion object keeps one class per name in the
  report, as Java does with static members.
