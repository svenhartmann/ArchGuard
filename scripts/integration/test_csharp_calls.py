#!/usr/bin/env python3
"""Real offline Roslyn regression. Requires a .NET SDK >= 8; no project build."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[2]
HELPER = REPO / "crates/archguard-engine/src/ArchguardCallScanner.cs"


def compile_helper(directory):
    assert HELPER.is_file(), "C# analysis needs a compiler-backed helper"
    sdks = []
    for line in subprocess.check_output(["dotnet", "--list-sdks"], text=True).splitlines():
        version, location = line.split(" [", 1)
        if int(version.split(".")[0]) >= 8:
            sdks.append((tuple(map(int, version.split("-")[0].split("."))), Path(location[:-1]) / version))
    assert sdks, "Install .NET SDK 8+ to run the Roslyn integration tests"
    sdk = max(sdks)[1]
    home = sdk.parent.parent
    major = int(sdk.name.split(".")[0])
    packs = sorted((home / "packs/Microsoft.NETCore.App.Ref").glob(f"{major}.*/ref/net{major}.0"))
    assert packs, "The SDK needs its reference pack"
    refs = list(packs[-1].glob("*.dll"))
    roslyn = sdk / "Roslyn/bincore"
    refs += [roslyn / "Microsoft.CodeAnalysis.dll", roslyn / "Microsoft.CodeAnalysis.CSharp.dll"]
    executable = directory / "scanner.dll"
    (directory / "sdk-version").write_text(sdk.name)
    subprocess.run(["dotnet", str(roslyn / "csc.dll"), "/nologo", "/noconfig", "/nostdlib+", "/target:exe", "/nullable:enable", "/out:" + str(executable), *["/reference:" + str(p) for p in refs], str(HELPER)], check=True, capture_output=True, text=True, timeout=45)
    executable.with_suffix(".runtimeconfig.json").write_text(json.dumps({"runtimeOptions": {"tfm": f"net{major}.0", "framework": {"name": "Microsoft.NETCore.App", "version": f"{major}.0.0"}}}))
    for name in ["Microsoft.CodeAnalysis.dll", "Microsoft.CodeAnalysis.CSharp.dll"]:
        shutil.copy(roslyn / name, directory / name)
    return executable, home


def decode_transport(raw):
    value=json.loads(raw)
    if 'encoding' not in value:return value
    assert value['encoding']=='string-references-v1'
    strings=value['strings']
    def expand(node):
        if isinstance(node,list):return [expand(item) for item in node]
        if isinstance(node,dict):
            if set(node)=={'$str'}:return strings[node['$str']]
            return {key:expand(item) for key,item in node.items()}
        return node
    return expand(value['analysis'])


def analyze(executable, home, root, sources, project=None, raw_output=False):
    manifest = root / "manifest.json"
    manifest.write_text(json.dumps({"root": str(root), "dotnet_root": str(home), "sdk_version": (executable.parent / "sdk-version").read_text(), "configuration": "Debug", "target_framework": None, "project": project,
        "sources": [{"path": str(root / name), "file": name, "graph_node": "mapped/" + name} for name in sources]}))
    result = subprocess.run(["dotnet", str(executable), str(manifest)], check=True, capture_output=True, text=True, timeout=45)
    return result.stdout if raw_output else decode_transport(result.stdout)


def main():
    with tempfile.TemporaryDirectory(prefix="archguard-cs-test-") as temporary:
        temp = Path(temporary)
        binary, home = compile_helper(temp)
        root = temp / "project"
        root.mkdir()
        sources = {
            "Api/Api.cs": """namespace Domain;
public interface IStore { void Save(string value); }
public partial class Store {
    public void Save(string value) {} public void Save(int value) {}
    public static void Clear() {}
}
public static class Utility {}
""",
            "Api/Other.cs": """namespace Domain;
public partial class Store : IStore { public class Nested { public void Ping() {} } }
""",
            "App/Client.cs": """using Domain;
namespace App;
public class Client {
    public void Run(IStore api, Store concrete) {
        api.Save("hello <ä>&</script>");
        concrete.Save(42);
        Store.Clear();
        var created = new Store();
        // concrete.Fake();
        var ignored = "api.Fake()";
#if EXTRA
        created.Save(7);
#endif
    }
}
""",
        }
        for name, code in sources.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(code)
        (root / "Api/Api.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
        (root / "App/App.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><DefineConstants>EXTRA</DefineConstants></PropertyGroup><ItemGroup><ProjectReference Include="../Api/Api.csproj" /></ItemGroup><Target Name="NeverExecute" BeforeTargets="Build"><Error Text="Project targets must not execute" /></Target></Project>')
        transport = analyze(binary, home, root, sources, raw_output=True)
        result = decode_transport(transport)
        assert "hello <ä>&</script>" in transport, "bounded helper JSON must preserve UTF-8 rather than inflate code with HTML escapes"
        assert result["status"] == "ready", result["diagnostics"]
        assert result["java_release"] == 0
        assert len([c for c in result["classes"] if c["name"] == "Domain.Store"]) == 1, "partial type declarations are one symbol"
        store = next(c for c in result["classes"] if c["name"] == "Domain.Store")
        contract = next(c for c in result["classes"] if c["name"] == "Domain.IStore")
        assert store["file"] == "Api/Api.cs"
        assert store.get("declaration_files") == ["Api/Api.cs", "Api/Other.cs"], "partial type locations must not be collapsed to the first file"
        assert store.get("supertype_sites") == [{"target": contract["id"], "file": "Api/Other.cs", "line": 2}], "base-clause evidence belongs to its actual partial declaration"
        assert {c["name"] for c in result["classes"]} == {"Domain.IStore", "Domain.Store", "Domain.Store.Nested", "Domain.Utility", "App.Client"}
        assert next(c for c in result["classes"] if c["name"] == "Domain.Utility")["is_abstract"] is False, "static utility classes are not abstract contracts"
        assert all(c["graph_node"].startswith("mapped/") for c in result["classes"])
        calls = result["calls"]
        assert len(calls) == 5, calls
        assert [c["dispatch"] for c in calls] == ["interface", "instance", "static", "constructor", "instance"]
        assert all(c["resolution"] == "project" for c in calls), calls
        assert "string" in calls[0]["to_method"] and "int" in calls[1]["to_method"], "actual overload targets are required"
        assert calls[0]["line"] == 5 and calls[0]["column"] == 9 and calls[0]["code"] == 'api.Save("hello <ä>&</script>")'
        assert all(c["from_method"] for c in calls)
        assert not list(root.rglob("*.dll")) and not list(root.rglob("obj")), "analysis must not build the project"
        print("C# calls: Roslyn targets, project references, overloads, partial/nested types, defines, source evidence and no project build passed")

        (root / "App/Client.cs").write_text('namespace App; public class Client { void Choose(string v) {} void Choose(int? v) {} void Run(dynamic d) { Choose(null); d.Unknown(); } }')
        ambiguous = analyze(binary, home, root, sources)
        assert ambiguous["status"] == "partial"
        assert len(ambiguous["calls"]) == 2
        assert all(c["resolution"] == "unresolved" and c["to_method"] is None and c["to_class"] is None for c in ambiguous["calls"])
        print("C# calls: ambiguous and dynamic dispatch remain explicitly unresolved")

        (root / "App/App.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net99.0</TargetFramework></PropertyGroup></Project>')
        unsupported = analyze(binary, home, root, sources)
        assert unsupported["status"] == "partial" and any("net99.0" in d for d in unsupported["diagnostics"])
        assert all(c["is_abstract"] is None for c in unsupported["classes"]), "partial source contexts cannot claim complete abstractness metrics"
        print("C# calls: unsupported target framework explains partial coverage")

        duplicate = temp / "duplicate"
        duplicate.mkdir()
        duplicate_sources = {}
        for folder in ["Left", "Right"]:
            (duplicate / folder).mkdir()
            (duplicate / folder / "Shared.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
            name = folder + "/Same.cs"
            duplicate_sources[name] = "namespace Same; class A { void Run() { Run(); } }"
            (duplicate / name).write_text(duplicate_sources[name])
        unique = analyze(binary, home, duplicate, duplicate_sources)
        assert unique["status"] == "ready", unique["diagnostics"]
        assert len(unique["classes"]) == 2 and len({c["id"] for c in unique["classes"]}) == 2, "same assembly/type names in independent projects remain distinct"
        assert {c["from_class"] for c in unique["calls"]} == {c["id"] for c in unique["classes"]}
        print("C# calls: independent projects with matching assembly/type names keep distinct identities")

        context = temp / "context"
        context.mkdir()
        (context / "Context.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
        (context / "A.cs").write_text("class A : IMissing { void Run() {} }")
        missing_type = analyze(binary, home, context, ["A.cs"])
        assert missing_type["status"] == "partial" and len(missing_type["classes"]) == 1, "missing external supertypes retain partial declarations"
        (context / "A.cs").write_text("class A { void Run() {\n#if NETCOREAPP3_0_OR_GREATER\nRun();\n#endif\n} }")
        defines = analyze(binary, home, context, ["A.cs"])
        assert defines["status"] == "ready" and len(defines["calls"]) == 1, "SDK framework symbols include earlier NETCOREAPP versions"
        (context / "B.cs").write_text("class B {}")
        (context / "Context.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework><EnableDefaultCompileItems>false</EnableDefaultCompileItems></PropertyGroup><ItemGroup><Compile Include="*.cs" Exclude="B.cs" /></ItemGroup></Project>')
        excluded = analyze(binary, home, context, ["A.cs", "B.cs"])
        assert excluded["files"] == 1 and [c["name"] for c in excluded["classes"]] == ["A"], "Compile Exclude belongs to its Include item"
        (context / "Context.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net7.0</TargetFramework></PropertyGroup></Project>')
        (context / "A.cs").write_text("class A { int[] values = [1, 2]; }")
        language = analyze(binary, home, context, ["A.cs"])
        assert language["status"] == "partial" and any("language version" in d.lower() or "C# 11" in d for d in language["diagnostics"]), "net7 defaults to C#11 and rejects C#12 collection expressions"
        (context / "global.json").write_text('{"sdk":{"version":"99.0.100","rollForward":"disable"}}')
        sdk = analyze(binary, home, context, ["A.cs"])
        assert sdk["status"] == "partial" and any("global.json" in d for d in sdk["diagnostics"]), "an unavailable pinned SDK cannot report complete coverage"
        print("C# calls: missing symbols, framework defines, Compile Exclude, TFM language defaults and SDK pins passed")
        (context / "global.json").unlink()
        (context / "Context.csproj").write_text('<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net8.0</TargetFramework></PropertyGroup></Project>')
        (context / "A.cs").write_text("class A { void One(){void Local(){B.First();}Local();} void Two(){void Local(){B.Second();}Local();} } class B { public static void First(){} public static void Second(){} }")
        local = analyze(binary, home, context, ["A.cs"])
        local_methods = [m for m in local["methods"] if m["name"] == "Local"]
        assert local["status"] == "ready" and len(local_methods) == 2 and len({m["id"] for m in local_methods}) == 2, "same-named local functions in separate methods have distinct identities"
        local_calls = [c for c in local["calls"] if c["code"].startswith("B.")]
        assert len({c["from_method"] for c in local_calls}) == 2, "local callers cannot be merged"
        print("C# calls: local function identities retain declaring method context")


if __name__ == "__main__":
    main()
