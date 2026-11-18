# ADR 0018: Swift support with ArchGuard's own token scanner

- **Status:** accepted
- **Date:** 2026-11-18

## Context

ArchGuard did not scan `.swift` files. Swift code should get the same import
graph and per-function metrics as PHP and Kotlin, which are measured by
ArchGuard's own token scanner without a language toolchain. A Swift toolchain
is not available on every machine (Linux CI, Windows), and running
`swift build` or the package manifest would execute repository code.

Swift differs from the JVM languages in two ways that matter here:

- A Swift file declares no package. A module is a build target: a Swift
  Package Manager target in `Package.swift`, or a target of an Xcode project.
  `import Core` names the target `Core`, never a file or type.
- Every file of a module sees every type of that module without an import.
  Which type a name means therefore depends on the whole module and its
  imported modules.

## Decision

1. **Modules are Swift Package Manager targets.** Every walked `Package.swift`
   is read as text with the Swift lexer (never run). Each `.target`,
   `.executableTarget`, `.testTarget` and `.macro` whose `name:` is a plain
   string literal declares a target. Its directory is the literal `path:`, else
   `Sources/<name>` (`Tests/<name>` for test targets), relative to the
   manifest. A computed name or path declares nothing.
2. **Imports link to the target directory.** `import X` (also `@testable`,
   `@_exported`, `public` and `import struct X.Y`) becomes a module-level
   symbol `X` without a stem. The pipeline binds it to the pseudo node of the
   directory of target `X`, as Go imports bind to their package directory.
   A name that no target or several targets declare is an external import
   with a warning. Toolchain and platform modules (`Foundation`, `UIKit`,
   `SwiftUI`, `XCTest`, `PackageDescription` and a few more) are ignored.
3. **Xcode projects are a gap.** `.xcodeproj` files are not read. Swift files
   outside every literal target form one module without a name; the call
   analysis reports how many such files there are.
4. **Files keep their path names.** Swift nodes are named by their (stripped)
   path, like Go and TypeScript. `Package.swift` is a Swift file and stays a
   node; its imports are platform modules.
5. **Declarations.** `class`, `struct`, `enum`, `actor` and `protocol` are
   classes with that kind; a protocol has kind `interface` and counts as
   abstract. Nested types get the id `file::Outer.Inner`. An `extension T` is
   first recorded as a class of kind `extension`; after all files are scanned,
   its functions and conformances move to the one project type whose path is
   `T`. With no or several such types, the extension stays its own class.
   The package of a class is its target name plus the directory inside the
   target (`Core/Model`), or the file's directory outside every target.
6. **Functions.** `func`, `init`, `deinit` and `subscript` with a body are
   measured. Protocol requirements, computed properties, accessors, property
   observers, property initializers and top-level statements are not. Local
   functions count toward their enclosing function; local types are skipped.
   `static func` and `class func` and top-level functions are static.
7. **Metrics** follow `ast-decisions-v1` and `source-maintainability/v1` as for
   Kotlin:
   - cyclomatic: `if`, `guard`, `for`, `while`, `repeat`, `catch`, each `case`
     label of a `switch` (not `default`, not `if case`), `&&`, `||`, `??` and
     the ternary `?`. A ternary `?` is told from an optional `?` by
     whitespace on both sides, which Swift requires for the ternary.
   - `if let` and `guard let` count like `if` and `guard`; the commas of a
     condition list are not decisions (a choice for simplicity: they behave
     like `&&` but share their token with argument lists).
   - cognitive: `if`, `guard`, loops, `switch`, `catch` and the ternary add 1
     plus nesting; `else` and `else if` add 1; the mandatory `else` of a
     `guard` adds nothing; `break label`/`continue label` add 1; `??`, `do`
     and `defer` add nothing.
   - nesting: control bodies, `switch` bodies and closures nest; `do` and
     `defer` blocks do not. `#if`/`#else` are directives, not decisions.
   - Halstead: a string with its interpolations is one operand.
8. **Calls only where syntax alone is safe.** A call site is a name directly
   followed by `(` in a measured body. `Type(…)`, `Type.init(…)`,
   `Type.Nested(…)` and `Type.staticMethod(…)` resolve when `Type` is exactly
   one top-level project type of the caller's own target or of a target the
   caller's file imports. A constructor resolves only for classes, structs
   and actors; a static call only when the type (with its extensions) has
   exactly one `static`/`class` function of that name. Everything else is
   `unresolved`: calls on values, `self` and `super`, bare lowercase calls
   (a function, an implicit `self` method or a closure), optional chains,
   overloads, generic `Type<T>(…)` and trailing-closure calls, which are not
   even recorded. Nothing is classified `external`, because a type name
   unknown to the project may still be a generic parameter or a type alias.
   The report stays `partial`.
9. **Lexing.** Swift comments nest; `"…"`, `"""…"""` and raw `#"…"#` strings
   hide their contents, interpolations `\(…)` (with nested strings) included.
   Regex literals (`/…/`, `#/…/#`) are not recognized.

## Alternatives

- **SourceKit-LSP or `swift-syntax` through a helper.** Precise, but needs a
  Swift toolchain on the machine and, for semantic resolution, a successful
  build of the package, which runs build plugins and macros. Rejected for the
  same reasons as for Kotlin; a later opt-in like the one for
  rust-analyzer (`ARCHGUARD_TRUST_PROJECT_COMPILER=1`) stays possible.
- **Read Xcode projects (`project.pbxproj`).** The format is an undocumented
  property list with file references by UUID. Left out until a real project
  needs it; the gap is reported instead.
- **Name a type `external` when the project does not declare it.** Rejected:
  generic parameters (`T()`) and type aliases look the same in syntax.
- **Treat every directory as a module.** Simpler, but it would invent
  dependencies between directories of one target and hide the real target
  boundaries that `import` expresses.

## Consequences

- Swift packages get an import graph between targets, class and function
  metrics and a small set of resolved calls without any toolchain.
- CBO, RFC, DIT and NOC stay partial for Swift; method-limit gates stay
  incomplete, as for PHP and Kotlin.
- Imports in Xcode-only projects stay unresolved, and their files are grouped
  by directory in the graph.
- A target name declared by two packages of one repository is ambiguous:
  its imports stay unresolved in the graph, and for calls both targets
  count as one module, so a type declared in both stays unresolved.
