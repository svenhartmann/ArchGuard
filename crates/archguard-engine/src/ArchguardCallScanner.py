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
"""
import ast
import json
import sys

ABSTRACT_BASES = {"ABC", "abc.ABC", "Protocol", "typing.Protocol", "typing_extensions.Protocol"}
ABSTRACT_META = {"ABCMeta", "abc.ABCMeta"}
ABSTRACT_DECORATORS = {"abstractmethod", "abc.abstractmethod"}
STATIC_DECORATORS = {"staticmethod", "classmethod"}
COMPREHENSIONS = (ast.ListComp, ast.SetComp, ast.DictComp, ast.GeneratorExp)


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


class Collector:
    def __init__(self, path, out):
        self.path = path
        self.package = path.rsplit("/", 1)[0] if "/" in path else ""
        self.out = out
        self.seen = set()

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

    def walk(self, statements, prefix, class_id):
        for node in statements:
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                owner = class_id or self.klass("(module)", "module", 1)
                self.function(owner, node, class_id is not None)
            elif isinstance(node, ast.ClassDef):
                name = prefix + node.name
                visibility = "private" if node.name.startswith("_") else "public"
                supertypes = [source(b) for b in node.bases]
                identity = self.klass(name, "class", node.lineno, visibility, is_abstract(node), supertypes)
                self.walk(node.body, name + ".", identity)
            else:
                # Conditional definitions: `if TYPE_CHECKING:`, `try: … except ImportError:`.
                for field in ("body", "orelse", "finalbody"):
                    self.walk(getattr(node, field, None) or [], prefix, class_id)
                for handler in getattr(node, "handlers", None) or []:
                    self.walk(handler.body, prefix, class_id)


def main():
    files = json.load(sys.stdin)["files"]
    out = {"version": sys.version.split()[0], "classes": [], "methods": [], "metrics": [], "diagnostics": []}
    for file in files:
        path = file["path"]
        try:
            tree = ast.parse(file["content"], filename=path)
            collected = {"classes": [], "methods": [], "metrics": []}
            Collector(path, collected).walk(tree.body, "", None)
        except SyntaxError as error:
            out["diagnostics"].append(f"{path}: line {error.lineno}: {error.msg}")
            continue
        except (ValueError, RecursionError, MemoryError) as error:
            out["diagnostics"].append(f"{path}: {type(error).__name__}: {error}")
            continue
        for key, values in collected.items():
            out[key].extend(values)
    json.dump(out, sys.stdout)


if __name__ == "__main__":
    main()
