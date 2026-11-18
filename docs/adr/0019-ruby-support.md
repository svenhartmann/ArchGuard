# ADR 0019: Ruby support with ArchGuard's own token scanner

- **Status:** accepted
- **Date:** 2026-11-18

## Context

Ruby projects (Rails apps, gems) had no import graph and no source metrics.
PHP and Kotlin showed the pattern that works without a language
installation: a lexer that shields comments and literals, a token scanner
for declarations and measures, and a static call resolver in the engine.
Ruby differs from both in three ways that shape the design:

- Blocks end with `end`, not braces, and `if`/`unless`/`while`/`until` and
  `rescue` also exist as modifiers without a body.
- Literals are hard to skip: heredocs whose body starts on the next line
  while the opening line goes on, `%w[…]`/`%i(…)`/`%q{…}` and other percent
  literals with nesting delimiters, regexes, symbols and `#{…}`
  interpolation that can contain strings again.
- Dependencies are often implicit. `require` names load-path files; Rails
  and Zeitwerk load classes by constant name without any `require`.

## Decision

1. **Files and packages.** `.rb` files are `Lang::Ruby` (node language `rb`,
   analyzer language `ruby`). A file declares no module; its package is its
   directory and its node is its path.
2. **Lexer** (`archguard-extract/src/lex/ruby.rs`). Comments (`#`,
   `=begin`/`=end`, data after `__END__`), strings with interpolation,
   percent literals, regexes, symbols and `?c` literals are literal tokens.
   A heredoc's body produces no tokens and is skipped at the end of the line
   that opened it. `/`, `%` and `<<` are literals only where an operand is
   expected (or after a name with a space before and none after, as in
   `split /,/`). Identifiers keep a trailing `?`/`!`; a label keeps its
   colon, so `if:` in `validates :x, if: :y` is no keyword.
3. **Import graph** (`archguard-engine/src/ruby_imports.rs`), on walked
   paths so that a derived source root such as `app` or `lib` does not hide
   them:
   - `require_relative 'x'` resolves against the requiring file.
   - `require 'a/b'` with one literal string resolves to `<lib>/a/b.rb` for
     every `lib/` directory of the project. Several matches: the `lib/` of
     the requiring file's own tree wins, otherwise none. Anything else
     (gems, the standard library) is an external import with a warning.
     Interpolated or computed arguments, `load`, `autoload` and
     `$LOAD_PATH` changes are not followed.
   - Constant references reach the engine as `TargetSpec::Qualified` with
     the lexical nesting as first segment. **Zeitwerk rule:** only when the
     project is a Rails app (a `config/application.rb`) or a gem that
     references `Zeitwerk::Loader`, a constant maps to a file by Zeitwerk
     naming: each `::` segment is a directory, CamelCase becomes snake_case
     as ActiveSupport's `underscore` does (`HTMLParser` → `html_parser`),
     and the file is looked up below the autoload roots: every `app/*`
     directory and every `app/*/concerns` of the Rails app, or the directory
     of the file that references `Zeitwerk::Loader`. Lexical scopes are
     tried innermost first (`Invoice` inside `module Billing` tries
     `billing/invoice.rb`, then `invoice.rb`). A constant found below two
     roots stays unresolved. Custom inflections, `autoload_paths`,
     `autoload_lib` and collapsed directories are not read. Without such a
     project, constant references stay unresolved and produce no warning.
4. **Declarations.** `class` (with a superclass that is a constant path;
   any other expression such as `Struct.new(:a)` makes it unknown),
   `module`, `def`, `def self.x`, `def`s in `class << self` and in
   ActiveSupport's `class_methods do`, endless `def x = …`, setter and
   operator methods. Names are qualified by lexical nesting
   (`Billing::Invoice`); a class reopened in one file is one class, a class
   declared in several files is ambiguous for the resolver. `include`,
   `prepend` and `extend` in a class body are supertypes, reported as
   project class ids where they resolve.
5. **Abstractness does not apply.** Ruby has no abstract classes or
   interfaces; `raise NotImplementedError` is a convention, not a
   declaration. Classes report `is_abstract: null`, and the abstraction
   metric of a scope that holds only Ruby files gives the reason "Ruby has
   no abstract classes: abstractness does not apply." instead of the
   generic unknown-coverage reason. Martin zones stay unmeasured for Ruby.
6. **Measures per method** (`ast-decisions-v1`, `source-maintainability/v1`,
   aligned with PHP and Kotlin):
   - cyclomatic +1 for `if`, `unless`, `elsif`, `while`, `until`, `for`
     (block and modifier forms), each `when`, each `in` of a `case`, each
     `rescue` (clause or modifier), the ternary `?`, `&&`, `||`, `and`,
     `or`, `||=`, `&&=` and `&.`.
   - cognitive: `if`, `unless`, loops, `case`, `rescue`, ternaries and
     modifiers add 1 plus nesting; `elsif` and the `else` of an `if` add 1;
     a run of logical operators adds 1 plus 1 per change of operator.
   - nesting: bodies of `if`/`unless`/loops/`case` and blocks (`do … end`,
     `{ … }`). Modifiers have no body. Rescue clauses do not nest their
     body because they have no delimiters of their own.
   - Halstead: keywords and operators are operators; names, constants,
     variables, `self`, `nil`, `true`, `false` and literals are operands;
     brackets, `,`, `;` and `end` are neither.
   - Unbalanced `end`s or braces: the file is reported and not measured.
7. **Calls** (`archguard-engine/src/semantic_calls/ruby_calls.rs`):
   - `Const.method` and `Const.new` on a project class declared in exactly
     one file, found by lexical lookup (innermost scope first; ancestors of
     the enclosing classes are not searched). `new` targets the class and
     its `initialize`, inherited ones included; a bare `new` in a class
     method constructs the class itself.
   - `self.method`, a bare `method` that is no local, and `super`, looked up
     as Ruby does: prepended modules, the class, included modules (last
     first), then the superclass; for class methods the singleton methods,
     extended modules and the superclass's class methods.
   - Unresolved: calls on variables, call results and other receivers,
     methods found nowhere (`method_missing`, `attr_*`, `delegate` and
     ActiveRecord attributes may answer them at runtime), classes declared
     in several files, constants assigned rather than declared
     (`Point = Struct.new`), and ancestors that are no unique project
     class. `send`, `public_send`, `__send__`, `method_missing`,
     `define_method`, `instance_eval`, `class_eval`, `eval`, `method` and
     similar reflective calls are unresolved with the label suffix
     `(dynamic)` and their own count in the coverage diagnostic.
   - External: a constant declared nowhere in a fully scanned project and
     not below a project namespace (`JSON.parse`).
   - Locals are a method's parameters, block parameters, `for` and
     `rescue => e` variables and every name assigned anywhere in the body.
     Calls in class bodies (`has_many`, `validates`), top-level code and
     inside `#{…}` are not counted.

## Alternatives

- **Run Ruby (Ripper, Prism or RuboCop).** Exact parsing, but it needs a
  Ruby installation and, for anything beyond syntax, loading the project's
  code. Rejected for the same reasons as for PHP: ArchGuard must not
  execute repository code, and the measures need syntax only.
- **Bundle a Ruby grammar (tree-sitter).** A new native dependency for one
  language, while the token scanner covers what the metrics and the
  resolver use. Can replace the lexer later without changing the symbols
  handed to the engine.
- **Resolve every constant by naming, also without Rails.** Plain Ruby
  projects do not load by name, so the edge would claim a dependency the
  runtime does not have. Rejected: unknown stays unknown.
- **Mark Ruby classes `is_abstract: false`.** It would make the Martin
  zones measurable, but it claims that a module meant as an interface is
  concrete. Rejected: the question does not apply.

## Consequences

- Ruby projects get an import graph, cycles, complexity, cognitive and
  Halstead metrics and a call graph without any tool installed.
- Rails apps get constant edges; plain Ruby projects get edges only from
  `require`/`require_relative` and from resolved calls and supertypes.
- Method-limit quality gates stay incomplete for Ruby, as for PHP and
  Kotlin, because syntax alone is no complete analysis.
- Metaprogramming (`define_method`, `method_missing`, DSLs, concerns'
  `included do` blocks) stays visible as unresolved or dynamic calls, never
  as invented edges.
