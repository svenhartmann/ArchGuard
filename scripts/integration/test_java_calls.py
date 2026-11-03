#!/usr/bin/env python3
"""Real JDK call-analysis regression; needs JDK 17+, not a browser or project build."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

HELPER = Path(__file__).resolve().parents[2] / "crates/archguard-engine/src/ArchguardCallScanner.java"


def analyze(sources, max_calls=None):
    with tempfile.TemporaryDirectory(prefix="archguard-java-test-") as temp:
        root = Path(temp)
        empty = root / "empty"
        empty.mkdir()
        manifest = []
        for name, code in sources.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(code)
            manifest.extend((str(path), name, name))
        manifest_path = root / "sources"
        manifest_path.write_text("\0".join(manifest) + "\0")
        command = ["java", "-Xmx512m", "--class-path", str(empty), str(HELPER), str(manifest_path), str(empty)]
        if max_calls is not None:
            context_path = root / "context"
            context_path.write_text("auto\0\0")
            command.extend([str(context_path), str(max_calls)])
        result = subprocess.run(
            command,
            capture_output=True, text=True, timeout=45, check=True,
            env={k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")},
        )
        assert not list(root.rglob("*.class")), "analyzed project classes must never be generated"
        return json.loads(result.stdout)


def test_local_constructor_survives_unrelated_type_errors():
    result = analyze({
        "p/Broken.java": "package p; class Broken {\n"
        + "\n".join(f"Missing{i} field{i};" for i in range(110)) + "\n}",
        "p/Module.java": "package p; class Module {}",
        "p/Factory.java": """package p;
class Factory {
    Object create() { return MissingGuice.create(new Module()); }
}
""",
    })
    assert result["status"] == "partial", "missing dependencies must retain partial coverage"
    assert len(result["diagnostics"]) <= 20, "more internal errors must not expand public diagnostic output"
    local = next(call for call in result["calls"] if call["code"] == "new Module()")
    assert (local["resolution"], local["to_class"], local["to_method"]) == (
        "project", "p.Module", "p.Module#<init>()"
    ), "unrelated missing types must not suppress a compiler-resolved local constructor"
    outer = next(call for call in result["calls"] if call["code"].startswith("MissingGuice.create"))
    assert outer["resolution"] == "unresolved" and outer["to_method"] is None
    assert not any("error limit reached" in message for message in result["diagnostics"])


def test_call_budget_keeps_earliest_sites_from_every_file():
    def file(name, sites):
        return (
            f"package p; class {name} {{ Helper helper; int n;\nvoid run() {{\n"
            + "\n".join("n = helper.get(n);" for _ in range(sites))
            + "\n}}}\n"
        )
    result = analyze(
        {"p/A.java": file("A", 60), "p/B.java": file("B", 60), "p/C.java": file("C", 3)},
        max_calls=20,
    )
    assert result["status"] == "partial"
    assert len(result["calls"]) == 20, "the reduced call budget must be exhausted exactly"
    per_file = {}
    for call in result["calls"]:
        per_file[call["file"]] = per_file.get(call["file"], 0) + 1
    assert per_file.get("p/C.java") == 3, "small files must keep every call site"
    assert set(per_file) == {"p/A.java", "p/B.java", "p/C.java"}, \
        "every scanned file must keep call sites instead of later files being dropped wholesale"
    assert all(count > 0 for count in per_file.values())
    assert any(message.startswith("Call sites are limited to 20;") for message in result["diagnostics"])
    assert result["details"]["enumeration_complete"] is False


def test_actual_diagnostic_cap_keeps_recovery_targets_unresolved():
    result = analyze({
        "p/Broken.java": "package p; class Broken {\n"
        + "\n".join(f"Missing{i} field{i};" for i in range(10_010)) + "\n}",
        "p/Factory.java": """package p;
class Module {
    void choose(String value) {}
    void choose(Integer value) {}
}
class Factory {
    void create() { new Module(); new Module().choose(null); }
}
""",
    })
    assert result["status"] == "partial"
    assert any("error limit reached" in message for message in result["diagnostics"])
    assert len(result["diagnostics"]) <= 20
    assert len(result["calls"]) == 3
    assert all(call["resolution"] == "unresolved" and call["to_class"] is None
               and call["to_method"] is None for call in result["calls"]), \
        "at the real compiler cap even otherwise valid calls must remain unresolved: diagnostics can be suppressed"


def test_many_missing_types_do_not_time_out_other_resolved_calls():
    sources = {}
    for index in range(80):
        sources[f"p/C{index}.java"] = (
            f"package p; class C{index} {{\n"
            + "\n".join(f"Missing{field} field{field};" for field in range(60))
            + "\nvoid target() {} void run() {\n"
            + "\n".join("target();" for _ in range(80)) + "\n}}"
        )
    sources["p/Ambiguous.java"] = (
        "package p; class Ambiguous { void choose(String x) {} "
        "void choose(Integer x) {} void run() { choose(null); } }"
    )
    # The same 45-second ceiling as the production backend. Missing libraries
    # must not turn thousands of otherwise resolved calls into an empty report.
    result = analyze(sources)
    assert result["status"] == "partial"
    assert len(result["classes"]) == 81
    assert len(result["calls"]) == 6401
    resolved = [c for c in result["calls"] if c["resolution"] == "project"]
    assert len(resolved) == 6400
    assert all(c["to_method"] == c["from_class"] + "#target()" for c in resolved)
    ambiguous = next(c for c in result["calls"] if c["from_class"] == "p.Ambiguous")
    assert ambiguous["resolution"] == "unresolved" and ambiguous["to_method"] is None
    assert len(result["diagnostics"]) <= 20


def main():
    source = {
        "p/Api.java": "package p; public interface Api { void send(String value); }",
        "p/Service.java": """package p;
    public class Service implements Api {
        public void send(String value) {}
        public void send(int value) {}
        public static void reset() {}
    }
    """,
        "p/Client.java": """package p;
    public class Client {
        public void run(Api api, Service service) {
            api.send("a");
            service.send(1);
            Service.reset();
            helper();
            // service.fake();
            String ignored = "service.fake()";
        }
        private void helper() {}
    }
    """,
    }
    result = analyze(source)
    assert result["status"] == "ready", result["diagnostics"]
    assert {c["id"] for c in result["classes"]} == {"p.Api", "p.Service", "p.Client"}, "actual class declarations are required"
    assert {m["id"] for m in result["methods"] if m["class_id"] == "p.Service"} == {
        "p.Service#send(java.lang.String)", "p.Service#send(int)", "p.Service#reset()"
    }, "overloads need distinct declared method identities; no synthetic constructor"
    calls = result["calls"]
    assert len(calls) == 4, calls
    assert [(c["to_method"], c["dispatch"], c["line"]) for c in calls] == [
        ("p.Api#send(java.lang.String)", "interface", 4),
        ("p.Service#send(int)", "instance", 5),
        ("p.Service#reset()", "static", 6),
        ("p.Client#helper()", "instance", 7),
    ], "calls must come from compiler targets, not imports or method-name guesses"
    assert all(c["from_method"] == "p.Client#run(p.Api,p.Service)" for c in calls)
    assert all(c["resolution"] == "project" and c["column"] > 0 for c in calls)
    assert calls[0]["code"] == 'api.send("a")', calls[0]
    print("Java calls: real compiler targets, overloads, interface/static/self calls and source evidence passed")

    ambiguous = analyze({"p/A.java": """package p;
    class A {
        void choose(String s) {}
        void choose(Integer i) {}
        void use() { choose(null); }
    }
    """})
    assert ambiguous["status"] == "partial"
    assert len(ambiguous["calls"]) == 1
    assert ambiguous["calls"][0]["resolution"] == "unresolved", "ambiguous overloads must not become guessed method targets"
    assert ambiguous["calls"][0]["to_method"] is None
    print("Java calls: ambiguous overload is explicitly unresolved")

    empty = analyze({"p/A.java": "package p; class A {} class B { B() {} }"})
    assert empty["calls"] == [], "javac's synthetic constructors and implicit super calls are not source calls"
    assert [m["id"] for m in empty["methods"]] == ["p.B#<init>()"]
    hierarchy = analyze({"p/Base.java": "package p; class Base { void inherited() {} }", "p/Leaf.java": """package p;
class Leaf extends Base {
    Base field = new Base();
    static { String.valueOf(1); }
    Leaf() {}
    void run() {
        inherited();
        String.valueOf(2);
    }
}
"""})
    assert hierarchy["status"] == "ready", hierarchy["diagnostics"]
    assert len(hierarchy["calls"]) == 4, hierarchy["calls"]
    constructor = next(c for c in hierarchy["calls"] if c["dispatch"] == "constructor")
    assert constructor["from_method"] is None and constructor["to_class"] == "p.Base"
    inherited = next(c for c in hierarchy["calls"] if c["target_label"] == "p.Base#inherited()")
    assert inherited["from_class"] == "p.Leaf" and inherited["resolution"] == "project"
    external = [c for c in hierarchy["calls"] if c["resolution"] == "external"]
    assert len(external) == 2 and all(c["to_method"] == "java.lang.String#valueOf(int)" for c in external)
    assert any(c["from_method"] is None for c in external), "static initializer calls have class context"
    missing = analyze({"p/A.java": "package p; class A { void run(Missing m) { m.call(); } }"})
    assert missing["status"] == "partial" and missing["diagnostics"]
    assert len(missing["calls"]) == 1 and missing["calls"][0]["resolution"] == "unresolved"
    assert missing["calls"][0]["to_method"] is None
    print("Java calls: no synthetic calls, inherited declarations, constructors, initializers, external and missing types passed")

    legacy = analyze({"p/Legacy.java": """package p;
class Legacy<E> extends java.util.AbstractList<E> {
    public boolean addFirst(E item) { return true; }
    public E get(int index) { return null; }
    public int size() { return 0; }
}
"""})
    assert legacy["status"] == "ready", "Java17 API baseline must not inherit Java21 List.addFirst conflicts: " + str(legacy["diagnostics"])
    assert legacy["java_release"] == 17, "the chosen API baseline is explicit in the payload"
    print("Java calls: explicit Java17 API baseline is stable across installed JDK versions")

    locals_result = analyze({"p/A.java": """package p;
class A {
    void one() { class Local { void call() {} } new Local().call(); }
    void two() { class Local { void call() {} } new Local().call(); }
}
"""})
    local_ids = [c["id"] for c in locals_result["classes"]]
    assert len(set(local_ids)) == len(local_ids), "local classes with identical simple names need distinct source identities"
    local_targets = [c["to_class"] for c in locals_result["calls"] if c["dispatch"] == "instance"]
    assert len(set(local_targets)) == 2, local_targets
    assert all(c["resolution"] == "project" for c in locals_result["calls"])
    print("Java calls: local classes retain distinct enclosing/source identities")

    test_local_constructor_survives_unrelated_type_errors()
    print("Java calls: unrelated missing types preserve resolved local constructors and bounded diagnostics")

    test_many_missing_types_do_not_time_out_other_resolved_calls()
    print("Java calls: many missing types retain thousands of resolved calls within the backend time limit")

    test_call_budget_keeps_earliest_sites_from_every_file()
    print("Java calls: a reduced call budget keeps the earliest sites of every file instead of dropping whole files")

    many_errors = analyze({"p/A.java": "package p; class A { void choose(String x) {} void choose(Integer x) {} void run() {\n"
        + "\n".join("choose(null);" for _ in range(110)) + "\n} }"})
    assert many_errors["status"] == "partial"
    assert len(many_errors["calls"]) == 110
    assert all(c["resolution"] == "unresolved" and c["to_method"] is None for c in many_errors["calls"]), "suppressed diagnostics at the compiler error cap must not promote recovery symbols to resolved calls"
    print("Java calls: compiler diagnostic limit cannot invent resolved targets")

    test_actual_diagnostic_cap_keeps_recovery_targets_unresolved()
    print("Java calls: actual diagnostic cap keeps all targets unresolved, including suppressed ambiguous overloads")

    generic = analyze({"p/Generic.java": """package p;
class Generic {
    <T extends Number> void pick(T value) {}
    <T extends CharSequence> void pick(T value) {}
    void run() { pick(1); pick("value"); }
}
"""})
    assert generic["status"] == "ready", generic["diagnostics"]
    assert len({m["id"] for m in generic["methods"]}) == len(generic["methods"]), "different generic bounds must not merge overload identities"
    assert {c["to_method"] for c in generic["calls"]} == {"p.Generic#pick(java.lang.Number)", "p.Generic#pick(java.lang.CharSequence)"}
    print("Java calls: erased parameter types distinguish generic overloads")


if __name__ == "__main__":
    main()
