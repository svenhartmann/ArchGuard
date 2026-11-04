// Roslyn evidence without running project targets, generators or application code.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Encodings.Web;
using System.Text.RegularExpressions;
using System.Xml;
using System.Xml.Linq;
using Microsoft.CodeAnalysis;
using Microsoft.CodeAnalysis.CSharp;
using Microsoft.CodeAnalysis.CSharp.Syntax;

sealed class ArchguardCallScanner
{
    const int MaxCalls = 100000, MaxDiagnostics = 40;
    readonly List<string> diagnostics = new();
    readonly Dictionary<string, Source> sources = new(StringComparer.Ordinal);
    readonly Dictionary<string, Project> projects = new(StringComparer.Ordinal);
    // Every parsed tree belongs to exactly one project. ClassId runs for each
    // type reference; scanning all projects' trees there was quadratic and
    // took minutes on dotnet/roslyn (13,600 files in 411 projects).
    readonly Dictionary<SyntaxTree, Project> treeOwners = new();
    readonly Dictionary<ITypeSymbol, string> classIds = new(SymbolEqualityComparer.Default);
    readonly Dictionary<string, Dictionary<string, object?>> classes = new(StringComparer.Ordinal);
    readonly Dictionary<string, Dictionary<string, object?>> methods = new(StringComparer.Ordinal);
    readonly List<Dictionary<string, object?>> calls = new();
    readonly List<Dictionary<string, object?>> typeUses = new();
    readonly Dictionary<string, Dictionary<string, object?>> classStates = new(StringComparer.Ordinal);
    string root = "", dotnetRoot = "", configuration = "Debug", sdkVersion = "";
    string? requestedFramework;
    bool partial;
    sealed record Source(string Path, string File, string GraphNode);
    sealed class Project
    {
        public string Path = "", Id = "", Framework = "", Assembly = "";
        public bool Partial, Visiting;
        public readonly List<Source> Sources = new();
        public readonly List<XElement> Items = new();
        public readonly Dictionary<string, string> Properties = new(StringComparer.OrdinalIgnoreCase);
        public CSharpCompilation? Compilation;
        public readonly Dictionary<SyntaxTree, Source> Trees = new();
        public readonly Dictionary<SyntaxTree, SemanticModel> Models = new();
        public readonly Dictionary<SyntaxTree, ErrorRanges> Errors = new();
    }
    sealed class ErrorRanges {
        readonly int[] starts, ends;
        public ErrorRanges(SemanticModel model) {
            var spans=model.GetDiagnostics().Where(d=>d.Severity==DiagnosticSeverity.Error).Select(d=>d.Location.SourceSpan).OrderBy(span=>span.Start).ToArray();
            starts=new int[spans.Length];ends=new int[spans.Length];
            for(int i=0;i<spans.Length;i++){starts[i]=spans[i].Start;ends[i]=Math.Max(i==0?-1:ends[i-1],spans[i].End);}
        }
        public bool Intersects(SyntaxNode node) {
            int low=0,high=starts.Length;
            while(low<high){int mid=(low+high)/2;if(starts[mid]<=node.Span.End)low=mid+1;else high=mid;}
            return low>0&&ends[low-1]>=node.Span.Start;
        }
    }
    static SemanticModel Model(Project p,SyntaxTree tree) {
        if(!p.Models.TryGetValue(tree,out var model)){model=p.Compilation!.GetSemanticModel(tree);p.Models[tree]=model;}
        return model;
    }
    static bool HasErrors(Project p,SyntaxNode node) {
        if(!p.Errors.TryGetValue(node.SyntaxTree,out var errors)){errors=new ErrorRanges(Model(p,node.SyntaxTree));p.Errors[node.SyntaxTree]=errors;}
        return errors.Intersects(node);
    }
    static Dictionary<string, object?> Map(params object?[] pairs)
    {
        var result = new Dictionary<string, object?>();
        for (int i = 0; i < pairs.Length; i += 2) result[(string)pairs[i]!] = pairs[i + 1];
        return result;
    }
    // Intern repeated source/symbol strings on the private process transport.
    // The engine expands them before exposing the ordinary public JSON model.
    static string Transport(object report) {
        var strings=new List<string>();var index=new Dictionary<string,int>(StringComparer.Ordinal);
        object? Encode(object? value) {
            if(value is string text) {
                if(text.Length<24)return text;
                if(!index.TryGetValue(text,out var id)){id=strings.Count;strings.Add(text);index[text]=id;}
                return Map("$str",id);
            }
            if(value is IDictionary<string,object?> dictionary)return dictionary.ToDictionary(pair=>pair.Key,pair=>Encode(pair.Value));
            if(value is System.Collections.IEnumerable items)return items.Cast<object?>().Select(Encode).ToArray();
            return value;
        }
        var encoded=Encode(report);
        return JsonSerializer.Serialize(Map("encoding","string-references-v1","strings",strings,"analysis",encoded),new JsonSerializerOptions { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
    }
    void Problem(string message, Project? project = null)
    {
        partial = true;
        if (project != null) project.Partial = true;
        if (diagnostics.Count < MaxDiagnostics && !diagnostics.Contains(message)) diagnostics.Add(message);
    }
    string Relative(string path) => System.IO.Path.GetRelativePath(root, path).Replace('\\', '/');
    bool Local(string path)
    {
        path = System.IO.Path.GetFullPath(path);
        if (path != root && !path.StartsWith(root + System.IO.Path.DirectorySeparatorChar, StringComparison.Ordinal)) return false;
        for (var cursor = path; cursor.Length > root.Length; cursor = System.IO.Path.GetDirectoryName(cursor)!)
            if (new FileInfo(cursor).LinkTarget != null) return false;
        return true;
    }
    static string? Text(JsonElement value, string key) => value.TryGetProperty(key, out var item) && item.ValueKind == JsonValueKind.String ? item.GetString() : null;
    public static int Main(string[] args)
    {
        Console.OutputEncoding = new System.Text.UTF8Encoding(false);
        // Roslyn binds `or` patterns and walks syntax recursively, partly on
        // pool threads. The launcher's DOTNET_DefaultStackSize sizes those and
        // this worker; the OS-sized main thread would overflow on long chains.
        int exit = 1;
        var worker = new System.Threading.Thread(() =>
        {
            try { new ArchguardCallScanner().Run(args[0]); exit = 0; }
            catch (Exception error) { Console.Error.WriteLine("C# analysis failed: " + error); }
        });
        worker.Start();
        worker.Join();
        return exit;
    }
    void Run(string manifest)
    {
        using var document = JsonDocument.Parse(File.ReadAllText(manifest));
        var input = document.RootElement;
        root = System.IO.Path.GetFullPath(Text(input, "root")!);
        dotnetRoot = Text(input, "dotnet_root")!;
        configuration = Text(input, "configuration") ?? "Debug";
        requestedFramework = Text(input, "target_framework");
        sdkVersion = Text(input, "sdk_version") ?? "";
        foreach (var value in input.GetProperty("sources").EnumerateArray())
        {
            var source = new Source(System.IO.Path.GetFullPath(Text(value, "path")!), Text(value, "file")!, Text(value, "graph_node")!);
            if (!Local(source.Path)) { Problem("Source outside the analysis root was excluded: " + source.File); continue; }
            sources[source.Path] = source;
        }
        var options = new EnumerationOptions { RecurseSubdirectories = true, AttributesToSkip = FileAttributes.ReparsePoint, IgnoreInaccessible = false };
        var paths = Directory.EnumerateFiles(root, "*.csproj", options)
            .Where(p => !Relative(p).Split('/').Any(s => s is ".git" or "bin" or "obj" or ".archguard"))
            .OrderBy(p => p, StringComparer.Ordinal).ToArray();
        foreach (var path in paths) LoadProject(path);
        foreach (var source in sources.Values)
        {
            var owners = projects.Values.Where(p => p.Path.Length > 0 && source.Path.StartsWith(System.IO.Path.GetDirectoryName(p.Path) + System.IO.Path.DirectorySeparatorChar, StringComparison.Ordinal))
                .OrderByDescending(p => System.IO.Path.GetDirectoryName(p.Path)!.Length).ThenBy(p => p.Path, StringComparer.Ordinal).ToArray();
            Project owner;
            if (owners.Length == 0)
            {
                if (!projects.TryGetValue("", out owner!))
                {
                    owner = new Project { Id = "source-only", Assembly = "ArchguardSources" };
                    projects[""] = owner;
                    Problem("No .csproj context for some sources; source-only C# analysis uses installed framework references.", owner);
                }
            }
            else
            {
                owner = owners[0];
                if (owners.Length > 1 && System.IO.Path.GetDirectoryName(owners[0].Path) == System.IO.Path.GetDirectoryName(owners[1].Path))
                    Problem("Multiple projects share a source directory; source ownership is ambiguous: " + source.File, owner);
            }
            if (Includes(owner, source.Path)) owner.Sources.Add(source);
        }
        var selected = Text(input, "project");
        IEnumerable<Project> active = projects.Values.Where(p => p.Sources.Count > 0).ToArray();
        if (selected != null)
        {
            var path = System.IO.Path.GetFullPath(System.IO.Path.Combine(root, selected));
            if (!Local(path) || !projects.TryGetValue(path, out var requested))
            {
                Problem("Configured C# project is missing or unsupported; select an existing .csproj within the root: " + selected);
                active = Array.Empty<Project>();
            }
            else active = new[] { requested };
        }
        foreach (var project in active) Compile(project);
        var analyzed = projects.Values.Where(p => p.Compilation != null).OrderBy(p => p.Id, StringComparer.Ordinal).ToArray();
        foreach (var project in analyzed) Declare(project);
        foreach (var project in analyzed) Details(project);
        foreach (var project in analyzed) Invocations(project);
        if (partial) foreach (var cls in classes.Values) cls["is_abstract"] = null;
        var analyzers = analyzed.Select(p => Map("language", "csharp", "backend", "roslyn", "status", p.Partial ? "partial" : "ready", "files", p.Trees.Count,
            "version", typeof(CSharpCompilation).Assembly.GetName().Version?.ToString(), "project", p.Id, "diagnostics", diagnostics.Where(d => d.Contains(p.Id)).ToArray())).ToArray();
        // The engine parses this bounded JSON transport before rendering HTML;
        // keep Unicode and generic/code symbols compact. HTML applies its own
        // script-safe escaping when the parsed model is embedded in a report.
        Console.WriteLine(Transport(Map("status", partial ? "partial" : "ready", "java_release", 0,
            "files", analyzed.Sum(p => p.Trees.Count), "classes", classes.Values.OrderBy(c => (string)c["id"]!, StringComparer.Ordinal),
            "methods", methods.Values.OrderBy(m => (string)m["id"]!, StringComparer.Ordinal),
            "calls", calls.OrderBy(c => (string)c["file"]!, StringComparer.Ordinal).ThenBy(c => (int)c["line"]!).ThenBy(c => (int)c["column"]!),
            "details", Map("enumeration_complete", calls.Count<MaxCalls && typeUses.Count<MaxCalls, "definition", "source-types-state/v1", "type_uses", typeUses, "class_states", classStates.Values, "method_metrics", sourceMetrics.Values, "inheritance", inheritance.Values),
            "diagnostics", diagnostics, "analyzers", analyzers)));
    }
    Project LoadProject(string path)
    {
        path = System.IO.Path.GetFullPath(path);
        if (projects.TryGetValue(path, out var existing)) return existing;
        var project = new Project { Path = path, Id = Relative(path), Assembly = System.IO.Path.GetFileNameWithoutExtension(path) };
        projects[path] = project;
        project.Properties["Configuration"] = configuration;
        project.Properties["MSBuildProjectDirectory"] = System.IO.Path.GetDirectoryName(path)!;
        if (!Local(path) || !File.Exists(path)) { Problem("Project reference is unavailable within the analysis root: " + project.Id, project); return project; }
        var directory = System.IO.Path.GetDirectoryName(path)!;
        CheckSdk(project, directory);
        for (var cursor = directory; cursor.Length >= root.Length; cursor = System.IO.Path.GetDirectoryName(cursor)!)
        {
            var props = System.IO.Path.Combine(cursor, "Directory.Build.props");
            if (File.Exists(props)) { ReadProjectXml(project, props); break; }
            if (cursor == root) break;
        }
        ReadProjectXml(project, path);
        project.Framework = requestedFramework ?? Get(project, "TargetFramework");
        if (project.Framework.Length == 0 && Get(project, "TargetFrameworks").Length > 0)
        {
            project.Framework = Get(project, "TargetFrameworks").Split(';')[0];
            Problem(project.Id + ": multiple target frameworks; analyzing " + project.Framework + ". Set semantic.csharp.target_framework to choose.", project);
        }
        project.Assembly = Get(project, "AssemblyName", project.Assembly);
        if (File.Exists(System.IO.Path.Combine(directory, "Directory.Build.targets")) || File.Exists(System.IO.Path.Combine(root, "Directory.Build.targets")))
            Problem(project.Id + ": Directory.Build.targets is not executed; generated/build-adjusted sources may be missing.", project);
        return project;
    }
    void CheckSdk(Project p, string directory)
    {
        for (var cursor = directory; cursor.Length >= root.Length; cursor = System.IO.Path.GetDirectoryName(cursor)!)
        {
            var path = System.IO.Path.Combine(cursor, "global.json");
            if (File.Exists(path))
            {
                if (!Local(path)) { Problem(p.Id + ": external global.json was excluded", p); return; }
                try
                {
                    using var document = JsonDocument.Parse(File.ReadAllText(path), new JsonDocumentOptions { CommentHandling = JsonCommentHandling.Skip, AllowTrailingCommas = true });
                    if (!document.RootElement.TryGetProperty("sdk", out var sdk)) return;
                    var requested = Text(sdk, "version");
                    if (requested == null) return;
                    if (!Version.TryParse(requested, out var version) || !Version.TryParse(sdkVersion, out var actual))
                    {
                        Problem(p.Id + ": global.json SDK " + requested + " could not be matched to analyzer SDK " + sdkVersion, p); return;
                    }
                    var policy = Text(sdk, "rollForward") ?? "patch";
                    bool allowed = policy switch {
                        "disable" => actual == version,
                        "patch" or "latestPatch" => actual >= version && actual.Major == version.Major && actual.Minor == version.Minor && actual.Build / 100 == version.Build / 100,
                        "feature" or "latestFeature" => actual >= version && actual.Major == version.Major && actual.Minor == version.Minor,
                        "minor" or "latestMinor" => actual >= version && actual.Major == version.Major,
                        "major" or "latestMajor" => actual >= version,
                        _ => false
                    };
                    if (!allowed) Problem(p.Id + ": global.json requests SDK " + requested + " (" + policy + "); analyzer uses installed SDK " + sdkVersion + ". Coverage is partial.", p);
                }
                catch (Exception error) { Problem(p.Id + ": global.json could not be read: " + error.Message, p); }
                return;
            }
            if (cursor == root) return;
        }
    }
    static string Get(Project p, string name, string fallback = "") => p.Properties.TryGetValue(name, out var value) ? value : fallback;
    string Expand(Project p, string value) => Regex.Replace(value, @"\$\(([^)]+)\)", match => p.Properties.TryGetValue(match.Groups[1].Value, out var replacement) ? replacement : match.Value);
    bool Condition(Project p, XElement element)
    {
        var condition = element.Attribute("Condition")?.Value;
        if (condition == null) return true;
        condition = Expand(p, condition).Trim();
        var comparison = Regex.Match(condition, "^['\"]([^'\"]*)['\"]\\s*(==|!=)\\s*['\"]([^'\"]*)['\"]$");
        if (comparison.Success && !condition.Contains("$("))
        {
            bool equal = String.Equals(comparison.Groups[1].Value, comparison.Groups[3].Value, StringComparison.OrdinalIgnoreCase);
            return comparison.Groups[2].Value == "==" ? equal : !equal;
        }
        Problem(p.Id + ": unsupported MSBuild condition: " + condition, p);
        return false;
    }
    void ReadProjectXml(Project p, string path)
    {
        if (!Local(path)) { Problem(p.Id + ": external/symlinked project metadata was excluded", p); return; }
        using var reader = XmlReader.Create(path, new XmlReaderSettings { DtdProcessing = DtdProcessing.Prohibit, XmlResolver = null });
        var document = XDocument.Load(reader);
        var project = document.Root!;
        var sdk = project.Attribute("Sdk")?.Value;
        if (path.EndsWith(".csproj") && sdk != "Microsoft.NET.Sdk" && sdk != "Microsoft.NET.Sdk.Web")
            Problem(p.Id + ": only Microsoft.NET.Sdk and Microsoft.NET.Sdk.Web project defaults are supported", p);
        if (sdk == "Microsoft.NET.Sdk.Web") p.Properties["ArchguardWebSdk"] = "true";
        p.Properties["MSBuildThisFileDirectory"] = System.IO.Path.GetDirectoryName(path)! + System.IO.Path.DirectorySeparatorChar;
        foreach (var group in project.Elements())
        {
            if (!Condition(p, group)) continue;
            if (group.Name.LocalName == "PropertyGroup") foreach (var property in group.Elements())
            {
                if (!Condition(p, property)) continue;
                p.Properties[property.Name.LocalName] = Expand(p, property.Value);
            }
            else if (group.Name.LocalName == "ItemGroup") foreach (var item in group.Elements())
            {
                if (!Condition(p, item)) continue;
                var clone = new XElement(item);
                foreach (var attr in clone.Attributes()) attr.Value = Expand(p, attr.Value);
                foreach (var child in clone.Elements()) child.Value = Expand(p, child.Value);
                p.Items.Add(clone);
            }
            else if (group.Name.LocalName is "Import" or "Choose" or "Sdk") Problem(p.Id + ": unsupported MSBuild " + group.Name.LocalName + "; context may be incomplete", p);
        }
    }
    bool Matches(Project p, string source, string pattern)
    {
        var directory = p.Path.Length > 0 ? System.IO.Path.GetDirectoryName(p.Path)! : root;
        var relative = System.IO.Path.GetRelativePath(directory, source).Replace('\\', '/');
        pattern = pattern.Replace('\\', '/');
        if (pattern.Contains("$(")) { Problem(p.Id + ": unresolved Compile pattern " + pattern, p); return false; }
        if (!pattern.Contains('*') && !pattern.Contains('?')) return System.IO.Path.GetFullPath(System.IO.Path.Combine(directory, pattern)) == source;
        var regex = "^" + Regex.Escape(pattern).Replace(@"\*\*/", "(?:.*/)?").Replace(@"\*\*", ".*").Replace(@"\*", "[^/]*").Replace(@"\?", "[^/]") + "$";
        return Regex.IsMatch(relative, regex);
    }
    bool Includes(Project p, string source)
    {
        bool included = Get(p, "EnableDefaultCompileItems", "true") != "false";
        foreach (var item in p.Items.Where(i => i.Name.LocalName == "Compile"))
        {
            bool excluded = (item.Attribute("Exclude")?.Value ?? "").Split(';', StringSplitOptions.RemoveEmptyEntries).Any(pattern => Matches(p, source, pattern));
            foreach (var pattern in (item.Attribute("Include")?.Value ?? "").Split(';', StringSplitOptions.RemoveEmptyEntries)) if (Matches(p, source, pattern) && !excluded) included = true;
            foreach (var pattern in (item.Attribute("Remove")?.Value ?? "").Split(';', StringSplitOptions.RemoveEmptyEntries)) if (Matches(p, source, pattern)) included = false;
        }
        return included;
    }
    IEnumerable<string> FrameworkReferences(Project p)
    {
        var framework = p.Framework;
        var match = Regex.Match(framework, @"^net([5-9]|[1-9][0-9]+)\.0$");
        var packsRoot = System.IO.Path.Combine(dotnetRoot, "packs", "Microsoft.NETCore.App.Ref");
        var available = Directory.Exists(packsRoot) ? Directory.GetDirectories(packsRoot).OrderByDescending(d => Version.TryParse(System.IO.Path.GetFileName(d), out var v) ? v : new Version()).ToArray() : Array.Empty<string>();
        var pack = match.Success ? available.FirstOrDefault(d => System.IO.Path.GetFileName(d).StartsWith(match.Groups[1].Value + ".", StringComparison.Ordinal)) : null;
        if (pack == null)
        {
            Problem(p.Id + ": reference pack unavailable for " + (framework.Length > 0 ? framework : "unspecified framework") + "; installed framework fallback is partial.", p);
            pack = available.FirstOrDefault();
        }
        if (pack == null) return Array.Empty<string>();
        var reference = Directory.GetDirectories(System.IO.Path.Combine(pack, "ref")).FirstOrDefault();
        var files = reference != null ? Directory.GetFiles(reference, "*.dll").ToList() : new List<string>();
        if (Get(p, "ArchguardWebSdk") == "true" || p.Items.Any(i => i.Name.LocalName == "FrameworkReference" && i.Attribute("Include")?.Value == "Microsoft.AspNetCore.App"))
        {
            var asp = System.IO.Path.Combine(dotnetRoot, "packs", "Microsoft.AspNetCore.App.Ref");
            var target = Directory.Exists(asp) ? Directory.GetDirectories(asp).Where(d => Directory.Exists(System.IO.Path.Combine(d, "ref", framework))).OrderByDescending(d => d).FirstOrDefault() : null;
            if (target != null) files.AddRange(Directory.GetFiles(System.IO.Path.Combine(target, "ref", framework), "*.dll"));
            else Problem(p.Id + ": ASP.NET reference pack unavailable for " + framework, p);
        }
        return files;
    }
    IEnumerable<string> PackageReferences(Project p)
    {
        var assets = System.IO.Path.Combine(p.Path.Length > 0 ? System.IO.Path.GetDirectoryName(p.Path)! : root, "obj", "project.assets.json");
        if (!p.Items.Any(i => i.Name.LocalName == "PackageReference")) return Array.Empty<string>();
        if (!File.Exists(assets) || !Local(assets)) { Problem(p.Id + ": NuGet dependency context missing (obj/project.assets.json); restore dependencies separately for full resolution.", p); return Array.Empty<string>(); }
        var result = new List<string>();
        try
        {
            using var document = JsonDocument.Parse(File.ReadAllText(assets));
            var data = document.RootElement;
            if (!data.GetProperty("targets").TryGetProperty(p.Framework, out var target)) { Problem(p.Id + ": assets do not contain target " + p.Framework, p); return result; }
            var folders = data.GetProperty("packageFolders").EnumerateObject().Select(f => f.Name).ToArray();
            foreach (var library in target.EnumerateObject())
            {
                if (!library.Value.TryGetProperty("compile", out var compile) || !data.GetProperty("libraries").TryGetProperty(library.Name, out var metadata) || Text(metadata, "type") != "package") continue;
                var path = Text(metadata, "path") ?? library.Name.ToLowerInvariant();
                foreach (var reference in compile.EnumerateObject())
                {
                    if (reference.Name.EndsWith("/_._")) continue;
                    var found = folders.Select(folder => System.IO.Path.Combine(folder, path, reference.Name)).FirstOrDefault(File.Exists);
                    if (found != null) result.Add(found); else Problem(p.Id + ": package reference missing: " + library.Name + "/" + reference.Name, p);
                }
            }
        }
        catch (Exception error) { Problem(p.Id + ": package context could not be read: " + error.Message, p); }
        return result;
    }
    CSharpCompilation? Compile(Project p)
    {
        if (p.Compilation != null) return p.Compilation;
        if (p.Visiting) { Problem(p.Id + ": cyclic project references; affected targets remain partial", p); return null; }
        p.Visiting = true;
        var references = new List<MetadataReference>();
        foreach (var path in FrameworkReferences(p).Concat(PackageReferences(p)).Distinct(StringComparer.Ordinal))
            try { references.Add(MetadataReference.CreateFromFile(path)); } catch (Exception error) { Problem(p.Id + ": invalid assembly reference: " + error.Message, p); }
        foreach (var item in p.Items)
        {
            var include = item.Attribute("Include")?.Value;
            if (item.Name.LocalName == "ProjectReference" && include != null)
            {
                if (item.Attribute("OutputItemType")?.Value == "Analyzer" || item.Attribute("ReferenceOutputAssembly")?.Value == "false") continue;
                var path = System.IO.Path.GetFullPath(System.IO.Path.Combine(System.IO.Path.GetDirectoryName(p.Path)!, include.Replace('\\', '/')));
                if (!Local(path) || !File.Exists(path) || include.Contains("$(")) { Problem(p.Id + ": project reference unavailable: " + include, p); continue; }
                var dependency = Compile(LoadProject(path));
                if (dependency != null) references.Add(dependency.ToMetadataReference());
            }
            else if (item.Name.LocalName == "Reference")
            {
                var hint = item.Elements().FirstOrDefault(e => e.Name.LocalName == "HintPath")?.Value;
                var path = hint == null ? null : System.IO.Path.GetFullPath(System.IO.Path.Combine(System.IO.Path.GetDirectoryName(p.Path)!, hint.Replace('\\', '/')));
                if (path != null && Local(path) && File.Exists(path)) references.Add(MetadataReference.CreateFromFile(path));
                else Problem(p.Id + ": assembly reference has no readable local HintPath: " + include, p);
            }
        }
        var versionText = Get(p, "LangVersion");
        var frameworkVersion = Regex.Match(p.Framework, @"^net([5-9]|[1-9][0-9]+)\.0$");
        if (versionText.Length == 0 && frameworkVersion.Success) versionText = (int.Parse(frameworkVersion.Groups[1].Value) + 4).ToString();
        var version = LanguageVersion.Default;
        if (versionText.Length > 0 && !LanguageVersionFacts.TryParse(versionText, out version)) Problem(p.Id + ": unsupported C# language version " + versionText, p);
        var constants = Get(p, "DefineConstants").Split(new[] { ';', ',' }, StringSplitOptions.RemoveEmptyEntries).Select(s => s.Trim()).ToList();
        constants.Add("TRACE"); if (configuration.Equals("Debug", StringComparison.OrdinalIgnoreCase)) constants.Add("DEBUG");
        if (frameworkVersion.Success)
        {
            int major = int.Parse(frameworkVersion.Groups[1].Value);
            constants.AddRange(new[] { "NET", "NETCOREAPP", "NET" + major + "_0", "NETCOREAPP3_1_OR_GREATER" });
            constants.AddRange(new[] { "NETCOREAPP1_0_OR_GREATER", "NETCOREAPP1_1_OR_GREATER", "NETCOREAPP2_0_OR_GREATER", "NETCOREAPP2_1_OR_GREATER", "NETCOREAPP2_2_OR_GREATER", "NETCOREAPP3_0_OR_GREATER" });
            for (int n = 5; n <= major; n++) constants.Add("NET" + n + "_0_OR_GREATER");
        }
        constants.RemoveAll(c => !SyntaxFacts.IsValidIdentifier(c));
        var parse = new CSharpParseOptions(version, preprocessorSymbols: constants.Distinct());
        foreach (var source in p.Sources.OrderBy(s => s.File, StringComparer.Ordinal))
        {
            var tree = CSharpSyntaxTree.ParseText(File.ReadAllText(source.Path), parse, source.Path);
            p.Trees[tree] = source;
            treeOwners[tree] = p;
        }
        var trees = p.Trees.Keys.ToList();
        if (Get(p, "ImplicitUsings") is "enable" or "true")
            trees.Add(CSharpSyntaxTree.ParseText("global using System; global using System.Collections.Generic; global using System.IO; global using System.Linq; global using System.Net.Http; global using System.Threading; global using System.Threading.Tasks;", parse));
        p.Compilation = CSharpCompilation.Create(p.Assembly, trees, references,
            new CSharpCompilationOptions(OutputKind.DynamicallyLinkedLibrary, allowUnsafe: Get(p, "AllowUnsafeBlocks") == "true", nullableContextOptions: Get(p, "Nullable") == "enable" ? NullableContextOptions.Enable : NullableContextOptions.Disable));
        foreach (var diagnostic in p.Compilation.GetDiagnostics().Where(d => d.Severity == DiagnosticSeverity.Error))
        {
            var span = diagnostic.Location.GetLineSpan();
            Problem(p.Id + ": " + (span.Path.Length > 0 ? Relative(span.Path) + ":" + (span.StartLinePosition.Line + 1) + ": " : "") + diagnostic.GetMessage(), p);
        }
        p.Visiting = false;
        return p.Compilation;
    }
    string TypeName(INamedTypeSymbol type) => type.OriginalDefinition.ToDisplayString(SymbolDisplayFormat.CSharpErrorMessageFormat);
    // Called only after every project compiled, so the cached ids are final.
    string ClassId(INamedTypeSymbol type)
    {
        type = type.OriginalDefinition;
        if (classIds.TryGetValue(type, out var cached)) return cached;
        return classIds[type] = ComputeClassId(type);
    }
    string ComputeClassId(INamedTypeSymbol type)
    {
        if (type.TypeKind == TypeKind.Error || type.ContainingAssembly == null) return "unresolved::" + TypeName(type);
        // Source declarations distinguish independent projects even when their
        // assembly identities and qualified names happen to be equal.
        var declarations = type.DeclaringSyntaxReferences
            .Select(r => treeOwners.TryGetValue(r.SyntaxTree, out var owner) ? owner : null)
            .OfType<Project>().Distinct().ToArray();
        if (declarations.Length == 1) return declarations[0].Id + "::" + TypeName(type);
        var matches = projects.Values.Where(p => p.Compilation?.Assembly.Identity.Equals(type.ContainingAssembly.Identity) == true).ToArray();
        var project = matches.Length == 1 ? matches[0].Id : "external:" + type.ContainingAssembly.Identity.ToString();
        return project + "::" + TypeName(type);
    }
    string MethodSignature(IMethodSymbol method)
    {
        method = (method.ReducedFrom ?? method).OriginalDefinition;
        var name = method.MethodKind == MethodKind.Constructor ? ".ctor" : method.Name;
        if (method.Arity > 0) name += Convert.ToChar(96) + method.Arity.ToString();
        return name + "(" + string.Join(",", method.Parameters.Select(p => (p.RefKind == RefKind.None ? "" : p.RefKind.ToString().ToLowerInvariant() + " ") + p.Type.ToDisplayString(SymbolDisplayFormat.CSharpErrorMessageFormat))) + ")";
    }
    string MethodId(IMethodSymbol method)
    {
        method = (method.ReducedFrom ?? method).OriginalDefinition;
        if (method.MethodKind == MethodKind.LocalFunction && method.ContainingSymbol is IMethodSymbol parent)
            return MethodId(parent) + "#local:" + MethodSignature(method) + "@" + (method.DeclaringSyntaxReferences.FirstOrDefault()?.Span.Start ?? 0);
        return ClassId(method.ContainingType) + "#" + MethodSignature(method);
    }
    void Declare(Project p)
    {
        foreach (var (tree, source) in p.Trees)
        {
            var model = Model(p,tree);
            foreach (var node in tree.GetRoot().DescendantNodes())
            {
                if (node is BaseTypeDeclarationSyntax || node is DelegateDeclarationSyntax)
                {
                    if (model.GetDeclaredSymbol(node) is not INamedTypeSymbol type) continue;
                    var id = ClassId(type);
                    if (classes.ContainsKey(id)) continue;
                    if (type.TypeKind != TypeKind.Interface) {
                        int depth=0;bool known=true;var seen=new HashSet<ISymbol>(SymbolEqualityComparer.Default);
                        for(var parent=type.BaseType;parent!=null;parent=parent.BaseType) {
                            if(parent.TypeKind==TypeKind.Error || !seen.Add(parent) || depth>=512){known=false;break;}depth++;
                        }
                        foreach(var reference in type.DeclaringSyntaxReferences)
                            if(reference.GetSyntax() is BaseTypeDeclarationSyntax declaration && declaration.BaseList!=null && HasErrors(p,declaration.BaseList))known=false;
                        inheritance[id]=Map("class_id",id,"base_class",type.BaseType==null?null:ClassId(type.BaseType),"depth",known?depth:null,"status",known?"ready":"partial");
                    }
                    var supers = type.Interfaces.Select(ClassId).ToList();
                    if (type.BaseType != null && type.BaseType.SpecialType != SpecialType.System_Object) supers.Add(ClassId(type.BaseType));
                    var superSites = new List<Dictionary<string, object?>>();
                    var declarationFiles = type.DeclaringSyntaxReferences
                        .Where(r => p.Trees.ContainsKey(r.SyntaxTree)).Select(r => p.Trees[r.SyntaxTree].File)
                        .Distinct(StringComparer.Ordinal).OrderBy(f => f, StringComparer.Ordinal).ToArray();
                    foreach (var reference in type.DeclaringSyntaxReferences)
                    {
                        if (reference.GetSyntax() is not BaseTypeDeclarationSyntax declaration || declaration.BaseList == null
                            || !p.Trees.TryGetValue(declaration.SyntaxTree, out var declaringSource)) continue;
                        var declaringModel = Model(p,declaration.SyntaxTree);
                        foreach (var clause in declaration.BaseList.Types)
                        {
                            if (declaringModel.GetTypeInfo(clause.Type).Type is not INamedTypeSymbol parent || parent.TypeKind == TypeKind.Error
                                || HasErrors(p,clause)) continue;
                            var parentId = ClassId(parent);
                            if (supers.Contains(parentId)) superSites.Add(Map("target", parentId, "file", declaringSource.File,
                                "line", declaration.GetLocation().GetLineSpan().StartLinePosition.Line + 1));
                        }
                    }
                    classes[id] = Map("id", id, "name", TypeName(type), "package", type.ContainingNamespace.IsGlobalNamespace ? "" : type.ContainingNamespace.ToDisplayString().Replace('.', '/'),
                        "file", source.File, "graph_node", source.GraphNode, "line", node.GetLocation().GetLineSpan().StartLinePosition.Line + 1,
                        "kind", type.TypeKind.ToString().ToLowerInvariant(), "is_abstract", type.IsAbstract, "visibility", type.DeclaredAccessibility.ToString().ToLowerInvariant(), "supertypes", supers.OrderBy(s => s, StringComparer.Ordinal),
                        "supertype_sites", superSites.OrderBy(s => (string)s["file"]!, StringComparer.Ordinal).ThenBy(s => (int)s["line"]!).ThenBy(s => (string)s["target"]!, StringComparer.Ordinal), "declaration_files", declarationFiles, "language", "csharp");
                }
                if (node is BaseMethodDeclarationSyntax || node is AccessorDeclarationSyntax || node is LocalFunctionStatementSyntax)
                {
                    if (model.GetDeclaredSymbol(node) is not IMethodSymbol method || method.ContainingType == null) continue;
                    var id = MethodId(method);
                    var declaration = Map("id", id, "class_id", ClassId(method.ContainingType), "name", method.Name, "signature", MethodSignature(method),
                        "file", source.File, "line", node.GetLocation().GetLineSpan().StartLinePosition.Line + 1,
                        "end_line", node.GetLocation().GetLineSpan().EndLinePosition.Line + 1, "is_static", method.IsStatic);
                    var complexity = MethodComplexity(node);
                    if (complexity != null) {
                        declaration["complexity"] = complexity;
                        var measured=new SourceMetrics();measured.Visit(MethodBody(node));sourceMetrics[id]=measured.Value(id);
                    }
                    // A partial definition without a body must not replace the
                    // measured implementation's source location and metrics.
                    if (method.PartialImplementationPart == null || !methods.ContainsKey(id)) methods[id] = declaration;
                }
            }
        }
    }
    static Dictionary<string, object?>? MethodComplexity(SyntaxNode declaration)
    {
        if (declaration.GetDiagnostics().Any(d => d.Severity == DiagnosticSeverity.Error)) return null;
        var body = MethodBody(declaration);
        if (body == null) return null;
        var metrics = new MethodMetrics();
        metrics.Visit(body);
        return Map("cyclomatic", metrics.Cyclomatic, "max_nesting", metrics.MaxNesting, "definition", "ast-decisions-v1");
    }
    static SyntaxNode? MethodBody(SyntaxNode declaration) => declaration switch
        {
            BaseMethodDeclarationSyntax method => (SyntaxNode?)method.Body ?? method.ExpressionBody?.Expression,
            AccessorDeclarationSyntax accessor => (SyntaxNode?)accessor.Body ?? accessor.ExpressionBody?.Expression,
            LocalFunctionStatementSyntax local => (SyntaxNode?)local.Body ?? local.ExpressionBody?.Expression,
            _ => null
        };
    readonly Dictionary<string,Dictionary<string,object?>> sourceMetrics=new(StringComparer.Ordinal),inheritance=new(StringComparer.Ordinal);
    sealed class SourceMetrics : CSharpSyntaxWalker {
        readonly HashSet<string> operators=new(),operands=new();readonly HashSet<int> lines=new();
        int n1,n2,cognitive,depth;
        public Dictionary<string,object?> Value(string id)=>Map("method_id",id,"definition","source-maintainability/v1","cognitive",cognitive,"distinct_operators",operators.Count,"distinct_operands",operands.Count,"operators",n1,"operands",n2,"source_lines",lines.Count);
        static bool Logical(SyntaxNode n)=>n.IsKind(SyntaxKind.LogicalAndExpression)||n.IsKind(SyntaxKind.LogicalOrExpression)||n.IsKind(SyntaxKind.AndPattern)||n.IsKind(SyntaxKind.OrPattern);
        void Tokens(SyntaxNode node){
            if(node is IdentifierNameSyntax || node is LiteralExpressionSyntax){operands.Add(node.ToString());n2++;}
            else if(node is ExpressionSyntax or StatementSyntax or CatchClauseSyntax && node is not BlockSyntax && node is not ParenthesizedExpressionSyntax){operators.Add(node.Kind().ToString());n1++;}
            else return;
            lines.Add(node.GetLocation().GetLineSpan().StartLinePosition.Line);
        }
        void Sequence(SyntaxNode node,List<int> sequence){
            while(node is ParenthesizedExpressionSyntax paren)node=paren.Expression;
            if(Logical(node)&&node is BinaryExpressionSyntax binary){Tokens(node);Sequence(binary.Left,sequence);sequence.Add(node.RawKind);Sequence(binary.Right,sequence);}
            else if(Logical(node)&&node is BinaryPatternSyntax pattern){Tokens(node);Sequence(pattern.Left,sequence);sequence.Add(node.RawKind);Sequence(pattern.Right,sequence);}
            else Visit(node);
        }
        void Branch(IfStatementSyntax node,bool ladder){
            Tokens(node);cognitive+=ladder?1:1+depth;Visit(node.Condition);depth++;Visit(node.Statement);depth--;
            if(node.Else?.Statement is IfStatementSyntax next)Branch(next,true);
            else if(node.Else!=null){cognitive++;depth++;Visit(node.Else.Statement);depth--;}
        }
        public override void Visit(SyntaxNode? node){
            if(node==null || node is AnonymousFunctionExpressionSyntax or LocalFunctionStatementSyntax or BaseTypeDeclarationSyntax)return;
            if(node is IfStatementSyntax branch){Branch(branch,false);return;}
            if(Logical(node)){var sequence=new List<int>();Sequence(node,sequence);int previous=-1;foreach(int op in sequence){if(op!=previous)cognitive++;previous=op;}return;}
            Tokens(node);
            bool nested=node is WhileStatementSyntax or DoStatementSyntax or ForStatementSyntax or CommonForEachStatementSyntax or CatchClauseSyntax or ConditionalExpressionSyntax or SwitchStatementSyntax or SwitchExpressionSyntax;
            if(nested){cognitive+=1+depth;depth++;}
            if(node is GotoStatementSyntax)cognitive++;
            base.Visit(node);if(nested)depth--;
        }
    }
    // Same source-rule contract as javac, with C#'s additional conditional
    // expressions. Nested function bodies have their own executable scope.
    sealed class MethodMetrics : CSharpSyntaxWalker
    {
        public int Cyclomatic = 1, MaxNesting;
        int depth;
        public override void Visit(SyntaxNode? node)
        {
            if (node == null || node is AnonymousFunctionExpressionSyntax or LocalFunctionStatementSyntax or BaseTypeDeclarationSyntax) return;
            bool control = node is IfStatementSyntax or WhileStatementSyntax or DoStatementSyntax or ForStatementSyntax
                or CommonForEachStatementSyntax or CatchClauseSyntax or ConditionalExpressionSyntax;
            if (control || node.IsKind(SyntaxKind.LogicalAndExpression) || node.IsKind(SyntaxKind.LogicalOrExpression)
                || node.IsKind(SyntaxKind.CoalesceExpression) || node.IsKind(SyntaxKind.CoalesceAssignmentExpression)
                || node.IsKind(SyntaxKind.AndPattern) || node.IsKind(SyntaxKind.OrPattern)
                || node is ConditionalAccessExpressionSyntax or WhenClauseSyntax or CatchFilterClauseSyntax
                or CaseSwitchLabelSyntax or CasePatternSwitchLabelSyntax
                || node is SwitchExpressionArmSyntax arm && arm.Pattern is not DiscardPatternSyntax) Cyclomatic++;
            bool nested = control || node is SwitchStatementSyntax or SwitchExpressionSyntax;
            if (node is IfStatementSyntax && node.Parent is ElseClauseSyntax) nested = false;
            if (nested) depth++;
            MaxNesting = Math.Max(MaxNesting, depth);
            base.Visit(node);
            if (nested) depth--;
        }
    }
    void Details(Project p)
    {
        foreach (var (tree, source) in p.Trees)
        {
            var model=Model(p,tree);
            var owners=new Dictionary<SyntaxNode,string>();
            var callers=new Dictionary<SyntaxNode,string?>();
            var fieldNames=new Dictionary<ISymbol,string>(SymbolEqualityComparer.Default);
            string FieldId(ISymbol field) {
                if(!fieldNames.TryGetValue(field,out var id)){id=ClassId(field.ContainingType)+"#field:"+field.Name;fieldNames[field]=id;}
                return id;
            }
            foreach (var node in tree.GetRoot().DescendantNodes().Where(n=>n is BaseTypeDeclarationSyntax or VariableDeclaratorSyntax or PropertyDeclarationSyntax or ParameterSyntax or MethodDeclarationSyntax or AttributeSyntax or IdentifierNameSyntax or AccessorDeclarationSyntax))
            {
                var ownerNode=node is BaseTypeDeclarationSyntax ? node : node.Ancestors().FirstOrDefault(n=>n is BaseTypeDeclarationSyntax);
                if(ownerNode==null)continue;
                if(!owners.TryGetValue(ownerNode,out var ownerId)) {
                    if(model.GetDeclaredSymbol(ownerNode) is not INamedTypeSymbol owner)continue;
                    ownerId=ClassId(owner);owners[ownerNode]=ownerId;
                }
                if(!classes.ContainsKey(ownerId))continue;
                if(!classStates.TryGetValue(ownerId,out var state)) {
                    state=Map("class_id",ownerId,"status","ready","fields",new List<Dictionary<string,object?>>(),"accesses",new List<Dictionary<string,object?>>());
                    classStates[ownerId]=state;
                }
                if(node==ownerNode && HasErrors(p,node))state["status"]="partial";
                var methodNode=node.AncestorsAndSelf().FirstOrDefault(n=>n is BaseMethodDeclarationSyntax or AccessorDeclarationSyntax or LocalFunctionStatementSyntax);
                string? methodId=null;
                if(methodNode!=null&&!callers.TryGetValue(methodNode,out methodId)){
                    methodId=model.GetDeclaredSymbol(methodNode) is IMethodSymbol method?MethodId(method):null;callers[methodNode]=methodId;
                }
                var fields=(List<Dictionary<string,object?>>)state["fields"]!;
                if(node is VariableDeclaratorSyntax && model.GetDeclaredSymbol(node) is IFieldSymbol field) {
                    fields.Add(Map("id",FieldId(field),"name",field.Name,"is_static",field.IsStatic,"file",source.File,"line",node.GetLocation().GetLineSpan().StartLinePosition.Line+1));
                    if(node.Parent is VariableDeclarationSyntax declaration) AddType(declaration.Type,"field");
                }
                if(node is PropertyDeclarationSyntax property && model.GetDeclaredSymbol(node) is IPropertySymbol prop) {
                    fields.Add(Map("id",FieldId(prop),"name",prop.Name,"is_static",prop.IsStatic,"file",source.File,"line",node.GetLocation().GetLineSpan().StartLinePosition.Line+1));
                    AddType(property.Type,"field");
                }
                if(node is ParameterSyntax parameter) AddType(parameter.Type,"parameter");
                if(node is MethodDeclarationSyntax declarationMethod) AddType(declarationMethod.ReturnType,"return");
                if(node is AttributeSyntax attribute) AddType(attribute.Name,attribute.Parent?.Parent is BaseTypeDeclarationSyntax?"class_annotation":"annotation");
                if(node is AccessorDeclarationSyntax accessor && methodId!=null && accessor.Parent?.Parent is PropertyDeclarationSyntax propertyOwner && model.GetDeclaredSymbol(propertyOwner) is IPropertySymbol accessorProperty) {
                    ((List<Dictionary<string,object?>>)state["accesses"]!).Add(Map("method",methodId,"field",FieldId(accessorProperty),"file",source.File,"line",node.GetLocation().GetLineSpan().StartLinePosition.Line+1));
                }
                if(node is IdentifierNameSyntax && methodId!=null && methods.ContainsKey(methodId)) {
                    var symbol=model.GetSymbolInfo(node);
                    if(symbol.CandidateReason==CandidateReason.None && symbol.Symbol is IFieldSymbol or IPropertySymbol && !HasErrors(p,node)) {
                        var accesses=(List<Dictionary<string,object?>>)state["accesses"]!;
                        if(accesses.Count<MaxCalls) accesses.Add(Map("method",methodId,"field",FieldId(symbol.Symbol!),"file",source.File,"line",node.GetLocation().GetLineSpan().StartLinePosition.Line+1));
                        else state["status"]="partial";
                    }
                }
                void AddType(TypeSyntax? syntax,string role) {
                    if(syntax==null)return;
                    foreach(var part in syntax.DescendantNodesAndSelf().OfType<TypeSyntax>()) {
                        if(part.Parent is QualifiedNameSyntax || part.Parent is AliasQualifiedNameSyntax)continue;
                        if(typeUses.Count>=MaxCalls){state["status"]="partial";Problem("Type-use evidence limited to "+MaxCalls,p);return;}
                        if(model.GetTypeInfo(part).Type is not INamedTypeSymbol target || target.TypeKind==TypeKind.Error || target.SpecialType!=SpecialType.None || HasErrors(p,part))continue;
                        var targetId=ClassId(target);
                        typeUses.Add(Map("from_class",ownerId,"from_method",methodId,"to_class",targetId,"target_label",TypeName(target),"role",role,"resolution",classes.ContainsKey(targetId)?"project":"external","file",source.File,"line",part.GetLocation().GetLineSpan().StartLinePosition.Line+1,"code",part.ToString()));
                    }
                }
            }
        }
    }
    void Invocations(Project p)
    {
        foreach (var (tree, source) in p.Trees)
        {
            var model = Model(p,tree);
            foreach (var node in tree.GetRoot().DescendantNodes().Where(n => n is InvocationExpressionSyntax or ObjectCreationExpressionSyntax or ImplicitObjectCreationExpressionSyntax or ConstructorInitializerSyntax))
            {
                if (calls.Count >= MaxCalls) { Problem("C# call sites limited to " + MaxCalls, p); return; }
                var ownerNode = node.Ancestors().FirstOrDefault(n => n is BaseTypeDeclarationSyntax);
                if (ownerNode == null || model.GetDeclaredSymbol(ownerNode) is not INamedTypeSymbol owner) { Problem(p.Id + ": top-level call sites are not represented as a declared class", p); continue; }
                var methodNode = node.Ancestors().FirstOrDefault(n => n is BaseMethodDeclarationSyntax or AccessorDeclarationSyntax or LocalFunctionStatementSyntax);
                var caller = methodNode != null ? model.GetDeclaredSymbol(methodNode) as IMethodSymbol : null;
                var info = model.GetSymbolInfo(node);
                var target = info.Symbol as IMethodSymbol;
                if (info.CandidateReason != CandidateReason.None || HasErrors(p,node) || target?.ContainingType.TypeKind == TypeKind.Error) target = null;
                var targetClass = target != null ? ClassId(target.ContainingType) : null;
                var resolution = target == null ? "unresolved" : classes.ContainsKey(targetClass!) ? "project" : "external";
                if (resolution == "unresolved") Problem(p.Id + ": some call targets are dynamic, ambiguous or unresolved", p);
                var location = node.GetLocation().GetLineSpan();
                var dispatch = target == null ? "unknown" : target.MethodKind == MethodKind.Constructor ? "constructor" : target.IsStatic ? "static" : target.ContainingType.TypeKind == TypeKind.Interface ? "interface" : "instance";
                calls.Add(Map("from_class", ClassId(owner), "from_method", caller != null ? MethodId(caller) : null,
                    "to_class", targetClass, "to_method", target != null ? MethodId(target) : null, "target_label", target != null ? TypeName(target.ContainingType) + "." + MethodSignature(target) : node.ToString(),
                    "dispatch", dispatch, "resolution", resolution, "file", source.File, "line", location.StartLinePosition.Line + 1, "column", location.StartLinePosition.Character + 1,
                    "code", node.ToString().Length > 1000 ? node.ToString()[..1000] : node.ToString()));
            }
        }
    }
}
