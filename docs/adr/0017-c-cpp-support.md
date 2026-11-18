# ADR 0017: C and C++ with ArchGuard's token scanner

- **Status:** accepted
- **Date:** 2026-11-18

## Context

ArchGuard measured no C or C++ code. PHP and Kotlin showed that a token
scanner without a compiler gives an import graph and per-function measures
(Complexity, Cognitive, Halstead) good enough for architecture review.
C and C++ add three problems the other languages do not have:

1. A `.h` header can be C or C++, and the file alone often does not say.
2. `#include` paths depend on include directories that only the build knows.
3. The preprocessor can change the code: `#if`/`#else` branches, macros.

## Decision

1. **One language for C and C++.** `Lang::Cpp` (`cpp` in
   `project.languages`, analyzer language `cpp`) covers `.c .h .cc .cpp .cxx
   .hpp .hh .hxx`. C is close enough to a subset of C++ for a token scanner,
   so `.h` needs no guess. The lexer is the C flavor of `lex.rs`.
2. **Includes.** `#include "x"` is a path spec: the including file's
   directory first, then the include directories. `#include <x>` uses only
   the include directories; if none has the file, it is a system or library
   header and stays external without a warning. An unresolved quoted
   include is a gap (an unresolved import warning). Includes in comments and
   strings never count; an include named by a macro is not reported.
3. **Include directories** come from `-I`, `-iquote`, `-isystem` and `/I`
   in `compile_commands.json` (root or `build/`; a relative `directory` is
   read against the file's directory) and from literal arguments of
   `include_directories` and `target_include_directories` in the
   `CMakeLists.txt` files of the root and of every directory above a C/C++
   source. `${CMAKE_CURRENT_SOURCE_DIR}` and `${CMAKE_CURRENT_LIST_DIR}` count
   as the list file's directory; arguments with other variables or generator
   expressions are skipped. The directories must lie inside the analyzed
   root. They are pooled for the whole repository, and a header found in two
   of them stays unresolved instead of guessing the compiler's order. They are
   part of the graph cache key.
4. **Package = directory.** Files declare no module.
5. **Declarations.** Classes, structs and unions with a body, namespaces
   (nested, inline and anonymous), free functions and methods with a body.
   Class ids are `<file>::<namespace>::<Class>`. A function defined inside a
   class body belongs to it; a free function to the file's `(module)`. An
   out-of-class definition `A::f` belongs to the project class whose
   qualified name is the current namespace plus `A`, else to the one class
   whose qualified name ends in `::A` (for `using namespace`), else to the
   file's `(module)`. A class is abstract when it declares a pure virtual
   function (`= 0`). Constructors and destructors are measured like methods.
   Enums, forward declarations, typedefs and anonymous structs are not
   classes.
6. **Measures.** The rules of the other token scanners
   (`ast-decisions-v1`, `source-maintainability/v1`): decisions are `if`,
   loops, `catch`, each `case`, `?`, `&&`/`and`, `||`/`or`; `switch` adds
   cognitive load with nesting but no decision; `goto` adds 1 cognitive;
   braces after `)`, `else`, `do`, `]` and `mutable` nest, plain blocks,
   `try` and initializer braces do not.
7. **Preprocessor.** Directive lines (with `\` continuations) are dropped.
   The code of all `#if`/`#elif`/`#else` branches is measured as written,
   at once; variants are not combined or chosen. A file whose branches
   leave brackets unbalanced gets the diagnostic "unbalanced brackets; not
   measured". Macro bodies are not measured; macro calls look like calls.
8. **No call or type resolution.** The call report stays `partial` with the
   fixed note `CallAnalyzer::CPP_TOKENS`.

## Alternatives

- **Separate `Lang::C` and `Lang::Cpp` with a `.h` heuristic** (C++ keywords
  in the header, or a sibling `.cpp`). Rejected: a wrong guess changes
  nothing measurable for a token scanner, but adds a rule users must learn.
- **libclang or a compiler run.** Rejected for this step: it needs an
  installed toolchain and a configured build, and runs project build logic.
  It is the path to resolved calls and types later.
- **Run the C preprocessor.** Rejected: one configuration hides the other
  branches, and the defines live in the build.
- **Per-file include order from `compile_commands.json`.** Rejected for now:
  the pooled set with "two candidates stay unresolved" never binds a wrong
  header, and CMake directory scopes would need a CMake evaluator.

## Consequences

- C and C++ projects get an import graph, cycles, layers, and class and
  function metrics without any tool.
- Projects without `compile_commands.json` or CMake (plain Makefiles) resolve
  only includes relative to the including file; includes through `-I` paths
  stay gaps. Generating `compile_commands.json` (CMake
  `-DCMAKE_EXPORT_COMPILE_COMMANDS=ON`, Bear) closes them.
- Code inside `#if 0` and in disabled platform branches is measured.
- Partial and explicit template specializations (`struct X<T*> { … }`) are
  skipped with their bodies.
- Known gaps of the lexer: C++ raw strings `R"(…)"` containing `"`, and
  digit separators `1'000`, can hide following code.
- CBO, RFC, DIT and NOC stay partial. Method-limit quality gates stay
  incomplete, as for the other token-scanned languages.
