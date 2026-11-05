"""ArchGuard Python source scanner. Parses with the standard ``ast`` module only;
project code is never imported or executed.

Reads ``{"files": [{"path", "content"}]}`` on stdin and writes classes, methods,
per-function measurements and diagnostics as JSON. The rules mirror
``rust_calls.rs`` (``ast-decisions-v1``, ``source-maintainability/v1``):

- cyclomatic: ``if``/``elif``, ``for``, ``while``, each ``except`` handler, each
  ``case`` except a final ``case _``, each case guard, each ``and``/``or``
  operand after the first, conditional expressions, and each comprehension
  ``for`` and ``if`` clause.
- cognitive: ``if``, loops, handlers, ``match`` and conditional expressions add
  1 plus nesting; ``elif`` and ``else`` add 1; a run of ``and``/``or`` adds 1
  plus 1 per change of operator. ``try`` and comprehensions add nothing.
- lambdas belong to their function one nesting level deeper; nested ``def`` and
  ``class`` bodies are their own scope and not measured; a leading docstring is
  not code.
- abstractness is syntactic: a class is abstract when it lists ``ABC`` or
  ``Protocol`` as a base, uses ``metaclass=ABCMeta`` or declares an
  ``@abstractmethod``.

Call targets are resolved statically and only where the source binds them:
module imports (absolute and relative, followed through re-exports), module
functions, classes (``C()`` targets ``C`` and its own ``__init__``), class
attributes ``C.m()``, ``self.m()``/``cls.m()`` and ``super().m()`` along the C3
order of project classes. A name rebound in its scope, defined twice or
assigned is not resolved. Any other receiver, a call result, a subscript or a
star import stays ``unresolved``; names outside the project are ``external``.
"""
import ast
import builtins
import io
import json
import sys

ABSTRACT_BASES = {"ABC", "abc.ABC", "Protocol", "typing.Protocol", "typing_extensions.Protocol"}
ABSTRACT_META = {"ABCMeta", "abc.ABCMeta"}
ABSTRACT_DECORATORS = {"abstractmethod", "abc.abstractmethod"}
STATIC_DECORATORS = {"staticmethod", "classmethod"}
COMPREHENSIONS = (ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)
OVERLOAD_DECORATORS = {"overload", "typing.overload", "typing_extensions.overload"}
# Bases without methods a project call could target.
EMPTY_BASES = {"builtins.object", "abc.ABC", "typing.Protocol", "typing_extensions.Protocol", "typing.Generic"}
BLOCK_FIELDS = {"body", "orelse", "finalbody", "handlers"}
BUILTIN_NAMES = set(dir(builtins))
OBJECT_METHODS = set(dir(object))
VALUE = ("value",)
AMBIGUOUS = ("ambiguous",)
UNKNOWN_BASE = ("unknown",)
UNRESOLVED = ("unresolved", "unknown", None, None)
CALL_LIMIT = 200_000
CODE_LIMIT = 200


def source(node):
    return ast.unparse(node)


class Measure(ast.NodeVisitor):
    def __init__(self):
        self.cyclomatic = 1
        self.cognitive = 0
        self.depth = 0
        self.max_nesting = 0
        self.operators = set()
        self.operands = set()
        self.operator_count = 0
        self.operand_count = 0
        self.lines = set()

    def operator(self, key, node):
        self.operators.add(key)
        self.operator_count += 1
        self.lines.add(node.lineno)

    def operand(self, key, node):
        self.operands.add(key)
        self.operand_count += 1
        self.lines.add(node.lineno)

    def all(self, nodes):
        for node in nodes:
            self.visit(node)

    def nested(self, nodes):
        self.depth += 1
        self.max_nesting = max(self.max_nesting, self.depth)
        self.all(nodes)
        self.depth -= 1

    def visit(self, node):
        handler = getattr(self, "visit_" + type(node).__name__, None)
        if handler:
            return handler(node)
        if isinstance(node, (ast.stmt, ast.expr)):
            self.operator(type(node).__name__, node)
        if isinstance(node, COMPREHENSIONS):
            for generator in node.generators:
                self.cyclomatic += 1 + len(generator.ifs)
        self.generic_visit(node)

    def branch(self, node, ladder):
        self.operator("If", node)
        self.cyclomatic += 1
        self.cognitive += 1 if ladder else 1 + self.depth
        self.max_nesting = max(self.max_nesting, self.depth + 1)
        self.visit(node.test)
        self.nested(node.body)
        rest = node.orelse
        # `elif` shares its `if` column; an `if` inside `else:` is indented.
        if len(rest) == 1 and isinstance(rest[0], ast.If) and rest[0].col_offset == node.col_offset:
            self.branch(rest[0], True)
        elif rest:
            self.cognitive += 1
            self.nested(rest)

    def visit_If(self, node):
        self.branch(node, False)

    def loop(self, node, header):
        self.operator(type(node).__name__, node)
        self.cyclomatic += 1
        self.cognitive += 1 + self.depth
        self.all(header)
        self.nested(node.body + node.orelse)

    def visit_For(self, node):
        self.loop(node, [node.target, node.iter])

    visit_AsyncFor = visit_For

    def visit_While(self, node):
        self.loop(node, [node.test])

    def visit_Try(self, node):
        self.operator("Try", node)
        self.all(node.body)
        for handler in node.handlers:
            self.operator("except", handler)
            self.cyclomatic += 1
            self.cognitive += 1 + self.depth
            if handler.type:
                self.visit(handler.type)
            if handler.name:
                self.operand(handler.name, handler)
            self.nested(handler.body)
        self.all(node.orelse + node.finalbody)

    visit_TryStar = visit_Try

    def visit_Match(self, node):
        self.operator("Match", node)
        self.cognitive += 1 + self.depth
        self.visit(node.subject)
        last = len(node.cases) - 1
        self.depth += 1
        self.max_nesting = max(self.max_nesting, self.depth)
        for index, case in enumerate(node.cases):
            pattern = case.pattern
            wildcard = isinstance(pattern, ast.MatchAs) and pattern.pattern is None and pattern.name is None
            self.cyclomatic += 0 if wildcard and index == last else 1
            self.cyclomatic += 1 if case.guard else 0
            self.visit(pattern)
            if case.guard:
                self.visit(case.guard)
            self.all(case.body)
        self.depth -= 1

    def visit_IfExp(self, node):
        self.operator("IfExp", node)
        self.cyclomatic += 1
        self.cognitive += 1 + self.depth
        self.max_nesting = max(self.max_nesting, self.depth + 1)
        self.visit(node.test)
        self.nested([node.body, node.orelse])

    def sequence(self, node, kinds):
        if not isinstance(node, ast.BoolOp):
            return self.visit(node)
        is_and = isinstance(node.op, ast.And)
        for index, value in enumerate(node.values):
            if index:
                self.operator("and" if is_and else "or", node)
                self.cyclomatic += 1
                kinds.append(is_and)
            self.sequence(value, kinds)

    def visit_BoolOp(self, node):
        kinds = []
        self.sequence(node, kinds)
        self.cognitive += 1 + sum(1 for a, b in zip(kinds, kinds[1:]) if a != b)

    def visit_Lambda(self, node):
        self.operator("Lambda", node)
        self.visit(node.args)
        self.nested([node.body])

    def visit_BinOp(self, node):
        self.operator(type(node.op).__name__, node)
        self.visit(node.left)
        self.visit(node.right)

    def visit_AugAssign(self, node):
        self.operator(type(node.op).__name__ + "=", node)
        self.visit(node.target)
        self.visit(node.value)

    def visit_UnaryOp(self, node):
        self.operator(type(node.op).__name__, node)
        self.visit(node.operand)

    def visit_Compare(self, node):
        for op in node.ops:
            self.operator(type(op).__name__, node)
        self.all([node.left] + node.comparators)

    def visit_Attribute(self, node):
        self.operator(".", node)
        self.visit(node.value)
        self.operand(node.attr, node)

    def visit_Name(self, node):
        self.operand(node.id, node)

    def visit_Constant(self, node):
        self.operand(repr(node.value), node)

    def visit_arg(self, node):
        self.operand(node.arg, node)

    def visit_keyword(self, node):
        if node.arg:
            self.operand(node.arg, node)
        self.visit(node.value)

    def visit_FunctionDef(self, node):
        pass

    visit_AsyncFunctionDef = visit_FunctionDef
    visit_ClassDef = visit_FunctionDef


def decorator_names(node):
    return {source(d.func if isinstance(d, ast.Call) else d) for d in node.decorator_list}


def is_abstract(node):
    bases = {source(b.value if isinstance(b, ast.Subscript) else b) for b in node.bases}
    meta = {source(k.value) for k in node.keywords if k.arg == "metaclass"}
    methods = [m for m in node.body if isinstance(m, (ast.FunctionDef, ast.AsyncFunctionDef))]
    return (
        bool(bases & ABSTRACT_BASES)
        or bool(meta & ABSTRACT_META)
        or any(decorator_names(m) & ABSTRACT_DECORATORS for m in methods)
    )


def module_name(path):
    """`pkg/m.py` -> (`pkg.m`, False); `pkg/__init__.py` -> (`pkg`, True)."""
    parts = (path[:-3] if path.endswith(".py") else path).split("/")
    if parts[-1] == "__init__":
        return ".".join(parts[:-1]), True
    return ".".join(parts), False


def bind(table, name, value):
    """A second, different binding of a name makes it ambiguous."""
    table[name] = value if table.get(name, value) == value else AMBIGUOUS


def header(node):
    """Child nodes of a statement outside its nested statement blocks."""
    parts = []
    for field, value in ast.iter_fields(node):
        if field not in BLOCK_FIELDS:
            parts.extend(value if isinstance(value, list) else [value])
    parts.extend(h.type for h in getattr(node, "handlers", None) or [] if h.type)
    return [part for part in parts if isinstance(part, ast.AST)]


def dotted(node):
    """`a.b.c` -> ["a", "b", "c"]; None unless a chain of names."""
    names = []
    while isinstance(node, ast.Attribute):
        names.append(node.attr)
        node = node.value
    return [node.id] + names[::-1] if isinstance(node, ast.Name) else None


def is_super(node):
    return isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "super"


def c3_merge(lines):
    """C3 linearization; None when the order is inconsistent."""
    lines = [list(line) for line in lines if line]
    order = []
    while lines:
        head = next((line[0] for line in lines if not any(line[0] in other[1:] for other in lines)), None)
        if head is None:
            return None
        order.append(head)
        lines = [rest for rest in ([x for x in line if x != head] for line in lines) if rest]
    return order


def stored_names(collector, body, imports):
    """Names a function body binds; its imports go into `imports`."""
    stores = set()
    for node in (child for statement in body for child in ast.walk(statement)):
        if isinstance(node, ast.Name) and not isinstance(node.ctx, ast.Load):
            stores.add(node.id)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            stores.add(node.name)
        elif isinstance(node, ast.arg):
            stores.add(node.arg)
        elif isinstance(node, ast.ExceptHandler) and node.name:
            stores.add(node.name)
        elif isinstance(node, (ast.Import, ast.ImportFrom)):
            collector.bind_imports(node, imports)
    return stores


def function_names(collector, func, class_id):
    """Locals, function-level imports and the receiver name of one function."""
    args = func.args
    positional = args.posonlyargs + args.args
    params = {a.arg for a in positional + args.kwonlyargs + [args.vararg, args.kwarg] if a}
    imports = {}
    stores = stored_names(collector, func.body, imports)
    receiver = None
    if class_id and positional and "staticmethod" not in decorator_names(func):
        receiver = positional[0].arg if positional[0].arg not in stores else None
    return {"locals": stores | params, "imports": imports, "self": receiver}


class Collector:
    def __init__(self, path, content, out):
        self.path = path
        self.package = path.rsplit("/", 1)[0] if "/" in path else ""
        self.dotted, self.is_package = module_name(path)
        self.rows = io.StringIO(content, newline=None).readlines()
        self.out = out
        self.seen = set()
        self.names = {}
        self.class_tables = {}
        # (from class or None for the module, from method, nodes, function, class)
        self.scopes = []

    def klass(self, name, kind, line, visibility=None, abstract=False, supertypes=()):
        identity = f"{self.path}::{name}"
        if identity not in self.seen:
            self.seen.add(identity)
            self.out["classes"].append({
                "language": "python", "id": identity, "name": name, "package": self.package,
                "file": self.path, "graph_node": self.path, "line": line, "kind": kind,
                "is_abstract": abstract, "visibility": visibility, "supertypes": sorted(supertypes),
            })
        return identity

    def function(self, class_id, node, in_class):
        identity = f"{class_id}#{node.name}@{node.lineno}"
        body = node.body
        if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) \
                and isinstance(body[0].value.value, str):
            body = body[1:]
        measure = Measure()
        measure.all(body)
        prefix = "async def" if isinstance(node, ast.AsyncFunctionDef) else "def"
        signature = f"{prefix} {node.name}({source(node.args)})"
        if node.returns:
            signature += f" -> {source(node.returns)}"
        self.out["methods"].append({
            "id": identity, "class_id": class_id, "name": node.name, "signature": signature,
            "file": self.path, "line": node.lineno, "end_line": node.end_lineno,
            "is_static": not in_class or bool(decorator_names(node) & STATIC_DECORATORS),
            "complexity": {"cyclomatic": measure.cyclomatic, "max_nesting": measure.max_nesting,
                           "definition": "ast-decisions-v1"},
        })
        self.out["metrics"].append({
            "method_id": identity, "definition": "source-maintainability/v1",
            "cognitive": measure.cognitive, "distinct_operators": len(measure.operators),
            "distinct_operands": len(measure.operands), "operators": measure.operator_count,
            "operands": measure.operand_count, "source_lines": len(measure.lines),
        })
        return identity

    def table(self, class_id):
        return self.class_tables[class_id]["names"] if class_id else self.names

    def walk(self, statements, prefix, class_id):
        for node in statements:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                self.define(node, class_id)
            elif isinstance(node, ast.ClassDef):
                self.class_def(node, prefix, class_id)
            else:
                self.statement(node, prefix, class_id)

    def define(self, node, class_id):
        owner = class_id or self.klass("(module)", "module", 1)
        identity = self.function(owner, node, class_id is not None)
        defaults = node.args.defaults + [d for d in node.args.kw_defaults if d]
        # Decorators and defaults run in the enclosing scope.
        self.scopes.append((class_id, None, node.decorator_list + defaults, None, None))
        self.scopes.append((owner, identity, node.body, node, class_id))
        decorators = decorator_names(node)
        if not decorators & OVERLOAD_DECORATORS:
            binding = ("function", owner, identity, "staticmethod" in decorators)
            bind(self.table(class_id), node.name, binding)

    def class_def(self, node, prefix, class_id):
        name = prefix + node.name
        visibility = "private" if node.name.startswith("_") else "public"
        supertypes = [source(b) for b in node.bases]
        identity = self.klass(name, "class", node.lineno, visibility, is_abstract(node), supertypes)
        self.class_tables[identity] = {"names": {}, "bases": node.bases, "module": self}
        outer = node.decorator_list + node.bases + [k.value for k in node.keywords]
        self.scopes.append((class_id, None, outer, None, None))
        bind(self.table(class_id), node.name, ("class", identity))
        self.walk(node.body, name + ".", identity)

    def statement(self, node, prefix, class_id):
        parts = header(node)
        self.scopes.append((class_id, None, parts, None, None))
        table = self.table(class_id)
        self.bind_imports(node, table)
        for name in (n.id for part in parts for n in ast.walk(part)
                     if isinstance(n, ast.Name) and not isinstance(n.ctx, ast.Load)):
            bind(table, name, VALUE)
        # Conditional definitions: `if TYPE_CHECKING:`, `try: … except ImportError:`.
        for field in ("body", "orelse", "finalbody"):
            self.walk(getattr(node, field, None) or [], prefix, class_id)
        for handler in getattr(node, "handlers", None) or []:
            if handler.name:
                bind(table, handler.name, VALUE)
            self.walk(handler.body, prefix, class_id)

    def bind_imports(self, node, table):
        if isinstance(node, ast.Import):
            for alias in node.names:
                head = alias.name.split(".")[0]
                bind(table, alias.asname or head, ("module", alias.name if alias.asname else head))
        elif isinstance(node, ast.ImportFrom):
            base = self.absolute(node)
            for alias in (a for a in node.names if a.name != "*"):
                bind(table, alias.asname or alias.name, ("symbol", base, alias.name))

    def absolute(self, node):
        """Module named by `from … import`, with relative levels applied."""
        if not node.level:
            return node.module
        parts = self.dotted.split(".") if self.dotted else []
        keep = max(len(parts) - node.level + (1 if self.is_package else 0), 0)
        return ".".join(parts[:keep] + ([node.module] if node.module else []))

    def text(self, node):
        """Source of a node, at most four lines and CODE_LIMIT characters."""
        last = min(node.end_lineno, node.lineno + 3)
        rows = [row.encode() for row in self.rows[node.lineno - 1:last]]
        if not rows:
            return source(node)
        if last == node.end_lineno:
            rows[-1] = rows[-1][:node.end_col_offset]
        rows[0] = rows[0][node.col_offset:]
        text = b"".join(rows).decode(errors="replace")
        text = text if last == node.end_lineno else text.rstrip() + " ..."
        return text if len(text) <= CODE_LIMIT else text[:CODE_LIMIT - 3] + "..."


class Resolver:
    def __init__(self, collectors, diagnostics):
        self.collectors = collectors
        self.diagnostics = diagnostics
        self.modules, self.packages = {}, set()
        for collector in collectors:
            name = collector.dotted
            # `m.py` beside `m/__init__.py`: the module is ambiguous.
            self.modules[name] = None if name in self.modules else collector
            parts = name.split(".")
            self.packages.update(".".join(parts[:i]) for i in range(1, len(parts)))
        self.classes = {k: v for c in collectors for k, v in c.class_tables.items()}
        self.order = {}
        self.count = 0

    def run(self):
        for collector in self.collectors:
            try:
                for scope in collector.scopes:
                    self.resolve_scope(collector, *scope)
            except RecursionError:
                collector.out["calls"].clear()
                self.diagnostics.append(f"{collector.path}: calls nested too deeply; none reported")
        if self.count > CALL_LIMIT:
            self.diagnostics.append(f"Call budget of {CALL_LIMIT} sites reached; later call sites are omitted.")

    def resolve_scope(self, collector, from_class, from_method, nodes, func, class_id):
        ctx = {"module": collector, "class_id": class_id, "self": None, "locals": set(), "imports": {}}
        if func is not None:
            ctx.update(function_names(collector, func, class_id))
        for node in (n for part in nodes for n in ast.walk(part) if isinstance(n, ast.Call)):
            self.count += 1
            if self.count > CALL_LIMIT:
                return
            resolution, dispatch, to_class, to_method = self.call(ctx, node.func)
            collector.out["calls"].append({
                "from_class": from_class or collector.klass("(module)", "module", 1),
                "from_method": from_method, "to_class": to_class, "to_method": to_method,
                "target_label": collector.text(node.func), "dispatch": dispatch,
                "resolution": resolution, "file": collector.path, "line": node.lineno,
                "column": node.col_offset + 1, "code": collector.text(node),
            })

    def lookup(self, ctx, name):
        if name in ctx["imports"]:
            return ctx["imports"][name]
        if name in ctx["locals"]:
            return VALUE
        if name in ctx["module"].names:
            return ctx["module"].names[name]
        return ("symbol", "builtins", name) if name in BUILTIN_NAMES else None

    def resolve(self, ctx, node):
        chain = dotted(node)
        return self.follow(self.lookup(ctx, chain[0]), chain[1:]) if chain else None

    def call(self, ctx, func):
        if isinstance(func, ast.Attribute) and ctx["class_id"]:
            if is_super(func.value):
                return self.member(ctx["class_id"], func.attr, True)
            if isinstance(func.value, ast.Name) and func.value.id == ctx["self"]:
                return self.member(ctx["class_id"], func.attr, False)
        found = self.resolve(ctx, func)
        kind = found[0] if found else None
        if kind == "function":
            return ("project", "static", found[1], found[2])
        if kind == "class":
            init = self.classes[found[1]]["names"].get("__init__") or VALUE
            return ("project", "constructor", found[1], init[2] if init[0] == "function" else None)
        if kind == "member":
            return self.member(found[1], found[2], False, "static")
        if kind == "external":
            owner = ".".join(found[1][:-1]) + "::(module)"
            return ("external", "unknown", owner, owner + "#" + found[1][-1])
        return UNRESOLVED

    def follow(self, binding, rest, depth=0):
        """The target a binding plus attribute chain names, or None."""
        kind = binding[0] if binding and depth < 16 else None
        if kind == "module":
            return self.into_module(binding[1], rest, depth)
        if kind == "symbol":
            return self.into_module(binding[1], [binding[2]] + rest, depth)
        if kind == "function" and not rest:
            return binding
        if kind == "class" and len(rest) > 1:
            return self.follow(self.classes[binding[1]]["names"].get(rest[0]), rest[1:], depth + 1)
        if kind == "class":
            return ("member", binding[1], rest[0]) if rest else binding
        return None

    def into_module(self, name, rest, depth):
        if not rest:
            return ("module", name)
        module = self.modules.get(name)
        if module and rest[0] in module.names:
            return self.follow(module.names[rest[0]], rest[1:], depth + 1)
        submodule = f"{name}.{rest[0]}"
        if submodule in self.modules or submodule in self.packages:
            return self.into_module(submodule, rest[1:], depth)
        if name in self.modules or name in self.packages:
            return None
        return ("external", name.split(".") + rest)

    def member(self, class_id, name, skip, dispatch="instance"):
        """`name` along the C3 order of `class_id`; only its own body if unknown."""
        order = self.mro(class_id)
        for owner in (order or [class_id])[1 if skip else 0:]:
            binding = self.classes[owner]["names"].get(name)
            if binding is None:
                continue
            if binding[0] != "function":
                return UNRESOLVED
            return ("project", "static" if binding[3] else dispatch, owner, binding[2])
        if order is not None and name in OBJECT_METHODS:
            return ("external", dispatch, "builtins::object", "builtins::object#" + name)
        return UNRESOLVED

    def mro(self, class_id):
        if class_id not in self.order:
            self.order[class_id] = None  # A cyclic hierarchy has no order.
            parents = [self.base(class_id, base) for base in self.classes[class_id]["bases"]]
            if UNKNOWN_BASE not in parents:
                parents = [p for p in parents if p]
                lines = [self.mro(parent) for parent in parents]
                if None not in lines:
                    self.order[class_id] = c3_merge([[class_id]] + lines + [parents])
        return self.order[class_id]

    def base(self, class_id, node):
        """Project class of a base, None for a method-free base, else unknown."""
        ctx = {"module": self.classes[class_id]["module"], "locals": set(), "imports": {}}
        found = self.resolve(ctx, node.value if isinstance(node, ast.Subscript) else node)
        if found and found[0] == "class":
            return found[1]
        if found and found[0] == "external" and ".".join(found[1]) in EMPTY_BASES:
            return None
        return UNKNOWN_BASE


def parse(file, diagnostics):
    path = file["path"]
    try:
        tree = ast.parse(file["content"], filename=path)
        collector = Collector(path, file["content"], {"classes": [], "methods": [], "metrics": [], "calls": []})
        collector.walk(tree.body, "", None)
        return collector
    except SyntaxError as error:
        diagnostics.append(f"{path}: line {error.lineno}: {error.msg}")
    except (ValueError, RecursionError, MemoryError) as error:
        diagnostics.append(f"{path}: {type(error).__name__}: {error}")
    return None


def main():
    files = json.load(sys.stdin)["files"]
    out = {"version": sys.version.split()[0], "classes": [], "methods": [], "metrics": [], "calls": [],
           "diagnostics": []}
    collectors = [c for c in (parse(file, out["diagnostics"]) for file in files) if c]
    Resolver(collectors, out["diagnostics"]).run()
    for collector in collectors:
        collector.out["calls"].sort(key=lambda call: (call["line"], call["column"]))
        for key, values in collector.out.items():
            out[key].extend(values)
    json.dump(out, sys.stdout)


if __name__ == "__main__":
    main()
