import com.sun.source.tree.*;
import com.sun.source.util.JavacTask;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.TreeScanner;
import com.sun.source.util.Trees;
import java.io.StringWriter;
import java.io.BufferedWriter;
import java.io.OutputStreamWriter;
import java.io.IOException;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.*;
import javax.lang.model.element.*;
import javax.tools.*;
import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilderFactory;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.xml.sax.SAXException;
import org.xml.sax.SAXParseException;
import org.xml.sax.helpers.DefaultHandler;

// This helper only parses and attributes explicitly supplied source files.
// Annotation processing and implicit source lookup are disabled. It never calls
// generate(), CompilationTask.call(), a project build, or application code.
class ArchguardCallScanner {
    static final int MAX_CALLS = 100_000;
    static final int MAX_DIAGNOSTICS = 20;
    // Keep enough internal diagnostics to reject recovery symbols at their
    // actual call sites even when project dependencies are unavailable. The
    // public diagnostic sample stays small. Hitting this larger, bounded cap
    // still makes every call unresolved: omitted errors cannot prove a target.
    static final int MAX_ERRORS = 10_000;
    static final class Source {
        final Path path;
        final String file, graphNode;
        Source(String path, String file, String graphNode) {
            this.path = Path.of(path).toAbsolutePath().normalize();
            this.file = file;
            this.graphNode = graphNode;
        }
    }
    final Map<Path, Source> sources = new HashMap<>();
    final List<Map<String, Object>> classes = new ArrayList<>();
    final List<Map<String, Object>> methods = new ArrayList<>();
    final List<Map<String, Object>> calls = new ArrayList<>();
    final List<Map<String, Object>> typeUses = new ArrayList<>();
    final List<Map<String, Object>> methodMetrics = new ArrayList<>();
    final List<Map<String, Object>> inheritance = new ArrayList<>();
    final Map<String, Map<String, Object>> classStates = new TreeMap<>();
    final Map<javax.lang.model.element.Element, String> fieldIds = new HashMap<>();
    final List<String> diagnostics = new ArrayList<>();
    final Map<javax.lang.model.element.Element, String> classIds = new HashMap<>();
    final Map<javax.lang.model.element.Element, String> methodIds = new HashMap<>();
    final Map<CompilationUnitTree, String> sourceText = new IdentityHashMap<>();
    final ErrorPositions errors = new ErrorPositions();
    final ErrorPositions syntaxErrors = new ErrorPositions();
    // Attribute each compiler diagnostic once. Call and method checks then
    // inspect only positions in their own file, not every error in the project.
    static final class ErrorPositions {
        int count;
        final Map<URI, NavigableSet<Long>> files = new HashMap<>();
        void add(Diagnostic<? extends JavaFileObject> problem) {
            count++;
            if (problem.getSource() != null)
                files.computeIfAbsent(problem.getSource().toUri(), key -> new TreeSet<>()).add(problem.getPosition());
        }
        boolean intersects(CompilationUnitTree unit, long start, long end) {
            // The global cap still invalidates all targets: suppressed errors
            // could belong to any file. Unknown positions invalidate that file.
            if (count >= MAX_ERRORS) return true;
            NavigableSet<Long> positions = files.get(unit.getSourceFile().toUri());
            if (positions == null) return false;
            if (positions.first() < 0) return true;
            Long position = positions.ceiling(start);
            return position != null && position <= end;
        }
    }
    Trees trees;
    final Set<CompilationUnitTree> unresolvedCrashes = new LinkedHashSet<>();
    JavacTask task;
    boolean partial;
    int release = 17;
    String classpath;
    String project;
    int maxCalls = MAX_CALLS;

    public static void main(String[] args) throws Exception {
        new ArchguardCallScanner().run(Path.of(args[0]), Path.of(args[1]), args.length > 2 ? Path.of(args[2]) : null,
            args.length > 3 ? Integer.parseInt(args[3]) : null);
    }

    void run(Path manifest, Path emptyPath, Path context, Integer maxCallsOverride) throws Exception {
        if (maxCallsOverride != null) {
            if (maxCallsOverride <= 0) throw new IllegalArgumentException("Call budget must be positive");
            maxCalls = maxCallsOverride;
        }
        classpath = emptyPath.toString();
        if (context != null) readContext(context);
        String[] fields = Files.readString(manifest, StandardCharsets.UTF_8).split("\u0000", -1);
        if (fields.length % 3 != 1 || !fields[fields.length - 1].isEmpty()) {
            throw new IllegalArgumentException("Invalid source manifest");
        }
        List<Path> paths = new ArrayList<>();
        for (int i = 0; i < fields.length - 1; i += 3) {
            Source source = new Source(fields[i], fields[i + 1], fields[i + 2]);
            sources.put(source.path, source);
            paths.add(source.path);
        }
        JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
        if (compiler == null) throw new IllegalStateException("A full JDK 17+ is required for Java call analysis");
        DiagnosticCollector<JavaFileObject> problems = new DiagnosticCollector<>();
        // Count eligible call sites per source first. A single global cap would
        // otherwise stop at the budget in manifest order and leave later files
        // without any call evidence. The quota keeps the earliest sites of every
        // file so coverage stays distributed within the same total budget.
        Invocations invocations = new Invocations();
        try (StandardJavaFileManager files = compiler.getStandardFileManager(problems, Locale.ROOT, StandardCharsets.UTF_8)) {
            // Use an existing empty directory, not an empty string (which can
            // mean the working directory), for both class and source lookup.
            List<String> options = List.of("--release", String.valueOf(release), "-proc:none", "-implicit:none", "-classpath", classpath,
                "-sourcepath", emptyPath.toString(), "-Xlint:none", "-encoding", "UTF-8", "-Xmaxerrs", String.valueOf(MAX_ERRORS));
            task = (JavacTask) compiler.getTask(new StringWriter(), files, problems, options, null,
                files.getJavaFileObjectsFromPaths(paths));
            List<CompilationUnitTree> units = new ArrayList<>();
            for (CompilationUnitTree unit : task.parse()) {
                units.add(unit);
                sourceText.put(unit, unit.getSourceFile().getCharContent(true).toString());
            }
            for (Diagnostic<? extends JavaFileObject> problem : problems.getDiagnostics())
                if (problem.getKind() == Diagnostic.Kind.ERROR) syntaxErrors.add(problem);
            // One analyze call per top-level type in todo order: a javac crash
            // (JDK 17-25 assert in Attr.visitYield under a missing anonymous
            // supertype) then costs that file instead of every call. In todo
            // order javac's class filter has nothing to defer and lose. Only
            // JavacTaskImpl offers per-type analysis (launched with
            // --add-exports jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED).
            var compilation = (com.sun.tools.javac.api.JavacTaskImpl) task;
            trees = Trees.instance(task);
            Set<CompilationUnitTree> crashed = new LinkedHashSet<>();
            for (javax.lang.model.element.Element type : compilation.enter()) {
                TreePath path = type instanceof TypeElement ? trees.getPath(type) : null;
                if (path == null || crashed.contains(path.getCompilationUnit())) continue;
                try {
                    compilation.analyze(List.of(type));
                } catch (RuntimeException | AssertionError crash) {
                    crashed.add(path.getCompilationUnit());
                }
            }
            units.removeAll(crashed);
            for (CompilationUnitTree unit : crashed) {
                partial = true;
                diagnostics.add(source(unit).file
                    + ": javac crashed while attributing this file; its classes, methods and calls are omitted.");
            }
            for (Diagnostic<? extends JavaFileObject> problem : problems.getDiagnostics()) {
                if (problem.getKind() == Diagnostic.Kind.ERROR) errors.add(problem);
            }
            if (errors.count >= MAX_ERRORS) {
                partial = true;
                diagnostics.add("Compiler error limit reached; all call targets remain unresolved because further diagnostics may be suppressed.");
            }
            for (CompilationUnitTree unit : units) new Declarations().scan(unit, null);
            for (CompilationUnitTree unit : units) new Details().scan(unit, null);
            invocations.counting = true;
            for (CompilationUnitTree unit : units) invocations.scan(unit, null);
            invocations.allocate();
            invocations.counting = false;
            for (CompilationUnitTree unit : units) invocations.scan(unit, null);
            for (CompilationUnitTree unit : unresolvedCrashes) {
                partial = true;
                diagnostics.add(source(unit).file
                    + ": javac crashed while resolving part of this file; the affected symbols stay unresolved.");
            }
        }
        for (Diagnostic<? extends JavaFileObject> diagnostic : problems.getDiagnostics()) {
            if (diagnostic.getKind() != Diagnostic.Kind.ERROR) continue;
            partial = true;
            if (diagnostics.size() >= MAX_DIAGNOSTICS) continue;
            String file = "Java source";
            if (diagnostic.getSource() != null) {
                Source source = sources.get(Path.of(diagnostic.getSource().toUri()).toAbsolutePath().normalize());
                if (source != null) file = source.file;
            }
            diagnostics.add(file + ":" + Math.max(1, diagnostic.getLineNumber()) + ": "
                + limit(diagnostic.getMessage(Locale.ROOT), 500));
        }
        if (invocations.omittedCallSites > 0) diagnostics.add("Call sites are limited to " + maxCalls
            + "; " + invocations.omittedCallSites + " call sites in " + invocations.truncatedFiles
            + " of " + invocations.counted.size() + " files are omitted; every scanned file keeps its earliest call sites. Analysis is partial.");
        classes.sort(Comparator.comparing(c -> (String)c.get("id")));
        methods.sort(Comparator.comparing(m -> (String)m.get("id")));
        calls.sort(Comparator.comparing((Map<String, Object> c) -> (String)c.get("file"))
            .thenComparingInt(c -> (Integer)c.get("line")).thenComparingInt(c -> (Integer)c.get("column")));
        String status = partial ? "partial" : "ready";
        var output = new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
        writeJson(map("status", status, "java_release", release, "files", sources.size(),
            "classes", classes, "methods", methods, "calls", calls, "diagnostics", diagnostics,
            "details", map("enumeration_complete", invocations.omittedCallSites == 0 && typeUses.size()<maxCalls, "definition", "source-types-state/v1", "type_uses", typeUses, "class_states", new ArrayList<>(classStates.values()), "method_metrics", methodMetrics, "inheritance", inheritance),
            "analyzers", List.of(map("language", "java", "backend", "javac", "status", status, "files", sources.size(),
                "version", Runtime.version().toString(), "project", project, "diagnostics", diagnostics))), output);
        output.newLine();
        output.flush();
    }

    // Metadata is read, never executed. Maven inheritance/profiles and Gradle
    // expressions are not evaluated; unhandled context is explicitly partial.
    void readContext(Path path) throws Exception {
        String[] fields = Files.readString(path, StandardCharsets.UTF_8).split("\u0000", -1);
        if (fields.length < 3 || !fields[fields.length - 1].isEmpty()) throw new IllegalArgumentException("Invalid compiler context");
        boolean explicit = !fields[0].equals("auto");
        release = explicit ? Integer.parseInt(fields[0]) : Runtime.version().feature();
        if (!fields[1].isEmpty()) classpath = fields[1];
        Set<Integer> releases = new TreeSet<>();
        Set<Path> buildDescriptors = new HashSet<>();
        for (int i = 2; i < fields.length - 1; i++) {
            Path descriptor = Path.of(fields[i]);
            buildDescriptors.add(descriptor.toAbsolutePath().normalize());
            if (project == null) project = descriptor.toString();
            if (descriptor.getFileName().toString().equals("pom.xml")) readMaven(descriptor, releases, !fields[1].isEmpty());
            else readGradle(descriptor, releases, !fields[1].isEmpty());
        }
        if (buildDescriptors.size() > 1) {
            partial = true;
            diagnostics.add("Multiple Java build contexts share this compiler scope; module dependencies and source-set boundaries are not evaluated. Resolved project targets do not prove visibility between independent modules.");
        }
        if (!explicit && releases.size() == 1) release = releases.iterator().next();
        else if (!explicit && releases.size() > 1) {
            partial = true;
            diagnostics.add("Multiple Java release settings detected (" + releases + "); using installed JDK " + release + ". Set semantic.java.release explicitly for this analysis scope.");
        }
    }
    static Element child(Element parent, String name) {
        if (parent == null) return null;
        for (Node node = parent.getFirstChild(); node != null; node = node.getNextSibling())
            if (node instanceof Element element && element.getTagName().equals(name)) return element;
        return null;
    }
    static String text(Element parent, String name) {
        Element element = child(parent, name);
        return element == null ? null : element.getTextContent().trim();
    }
    Integer parseRelease(String value, Map<String, String> properties) {
        if (value == null) return null;
        Set<String> visited = new HashSet<>();
        while (value.startsWith("${") && value.endsWith("}") && visited.add(value)) {
            String resolved = properties.get(value.substring(2, value.length() - 1));
            if (resolved == null) break;
            value = resolved.trim();
        }
        if (value.startsWith("1.")) value = value.substring(2);
        if (value.matches("[0-9]{1,3}")) return Integer.parseInt(value);
        partial = true;
        diagnostics.add("Java release expression could not be resolved statically; set semantic.java.release explicitly.");
        return null;
    }
    void readMaven(Path descriptor, Set<Integer> releases, boolean explicitClasspath) {
        try {
            DocumentBuilderFactory factory = DocumentBuilderFactory.newInstance();
            factory.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
            factory.setFeature("http://xml.org/sax/features/external-general-entities", false);
            factory.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
            factory.setXIncludeAware(false);factory.setExpandEntityReferences(false);
            factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "");
            factory.setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "");
            var builder = factory.newDocumentBuilder();
            builder.setErrorHandler(new DefaultHandler() {
                @Override public void error(SAXParseException e) throws SAXException { throw e; }
                @Override public void fatalError(SAXParseException e) throws SAXException { throw e; }
            });
            Element root = builder.parse(descriptor.toFile()).getDocumentElement();
            Map<String, String> properties = new HashMap<>();
            Element propertyNode = child(root, "properties");
            if (propertyNode != null) for (Node n = propertyNode.getFirstChild(); n != null; n = n.getNextSibling())
                if (n instanceof Element element) properties.put(element.getTagName(), element.getTextContent().trim());
            String configured = properties.get("maven.compiler.release");
            if (configured == null) configured = properties.get("maven.compiler.source");
            Element build = child(root, "build");
            // Managed defaults apply to the lifecycle compiler plugin. Explicit
            // build/plugins configuration is applied afterwards and overrides it.
            for (Element plugins : new Element[] {child(child(build, "pluginManagement"), "plugins"), child(build, "plugins")}) {
                if (plugins == null) continue;
                for (Node n = plugins.getFirstChild(); n != null; n = n.getNextSibling()) {
                    if (!(n instanceof Element plugin) || !"maven-compiler-plugin".equals(text(plugin, "artifactId"))) continue;
                    Element configuration = child(plugin, "configuration");
                    String pluginRelease = text(configuration, "release");
                    if (pluginRelease == null) pluginRelease = text(configuration, "source");
                    if (pluginRelease != null) configured = pluginRelease;
                }
            }
            Integer parsed = parseRelease(configured, properties);
            if (parsed != null) releases.add(parsed);
            if (child(root, "profiles") != null || child(root, "parent") != null) {
                partial = true;
                diagnostics.add("Maven parent/profile configuration is not evaluated: " + descriptor.getFileName() + ". Explicit release and classpath settings cover only this analysis scope.");
            }
            if (!explicitClasspath && child(root, "dependencies") != null) {
                partial = true;
                diagnostics.add("Maven dependencies are not resolved or downloaded. Configure semantic.java.classpath with existing JARs or class directories.");
            }
        } catch (Exception error) {
            partial = true;
            diagnostics.add("Maven configuration could not be parsed safely: " + descriptor.getFileName() + ". Set semantic.java.release and classpath explicitly.");
        }
    }
    void readGradle(Path descriptor, Set<Integer> releases, boolean explicitClasspath) throws Exception {
        String raw = Files.readString(descriptor, StandardCharsets.UTF_8);
        String source = gradleCode(raw);
        Set<Integer> apiReleases = new TreeSet<>(), sourceReleases = new TreeSet<>(), toolchains = new TreeSet<>();
        var toolchain = java.util.regex.Pattern.compile("languageVersion(?:\\s*\\.\\s*set)?\\s*(?:=|\\()\\s*JavaLanguageVersion\\s*\\.\\s*of\\s*\\(\\s*([0-9]+)\\s*\\)").matcher(source);
        while (toolchain.find()) toolchains.add(Integer.parseInt(toolchain.group(1)));
        var api = java.util.regex.Pattern.compile("options\\s*\\.\\s*release(?:\\s*\\.\\s*set)?\\s*(?:=|\\()\\s*([0-9]+)\\s*(?=[)\\r\\n;}]|$)").matcher(source);
        while (api.find()) apiReleases.add(Integer.parseInt(api.group(1)));
        var compatible = java.util.regex.Pattern.compile("sourceCompatibility\\s*(?:=|\\()\\s*(?:JavaVersion\\.VERSION_)?[\\\"']?([0-9]+(?:[._][0-9]+)?)[\\\"']?\\s*(?=[)\\r\\n;}]|$)").matcher(source);
        while (compatible.find()) {
            Integer parsed = parseRelease(compatible.group(1).replace('_', '.'), Map.of());
            if (parsed != null) sourceReleases.add(parsed);
        }
        boolean apiMentioned = source.matches("(?s).*options\\s*\\.\\s*release.*");
        if (apiMentioned && apiReleases.isEmpty() || !apiMentioned && source.contains("sourceCompatibility") && sourceReleases.isEmpty()
            || !apiMentioned && !source.contains("sourceCompatibility") && source.contains("languageVersion") && toolchains.isEmpty()) {
            partial = true;
            diagnostics.add("Gradle Java configuration uses an unsupported expression; set semantic.java.release explicitly.");
        }
        releases.addAll(!apiReleases.isEmpty() ? apiReleases : !sourceReleases.isEmpty() ? sourceReleases : toolchains);
        if (source.contains("apply") || source.contains("subprojects") || source.contains("allprojects") || source.contains("libs.versions")) {
            partial = true;
            diagnostics.add("Gradle applied scripts, convention plugins and multi-project configuration are not evaluated.");
        }
        if (!explicitClasspath && source.contains("dependencies")) {
            partial = true;
            diagnostics.add("Gradle dependencies are not resolved or downloaded. Configure semantic.java.classpath with existing JARs or class directories.");
        }
    }
    // Keep numeric string literals used by Groovy sourceCompatibility, while
    // removing comments and all other string contents before matching settings.
    static String gradleCode(String source) {
        StringBuilder result = new StringBuilder();
        for (int i = 0; i < source.length();) {
            int start = i;
            if (source.startsWith("//", i)) {
                i += 2;while (i < source.length() && source.charAt(i) != '\n') i++;
            } else if (source.startsWith("/*", i)) {
                int end = source.indexOf("*/", i + 2);i = end < 0 ? source.length() : end + 2;
            } else if (source.charAt(i) == '\'' || source.charAt(i) == '"') {
                char quote = source.charAt(i);String delimiter = String.valueOf(quote);
                if (source.startsWith(delimiter.repeat(3), i)) delimiter = delimiter.repeat(3);
                i += delimiter.length();int content = i;
                while (i < source.length() && !source.startsWith(delimiter, i)) {
                    if (source.charAt(i) == '\\') i = Math.min(source.length(), i + 2);else i++;
                }
                String value = source.substring(content, i);
                i = Math.min(source.length(), i + delimiter.length());
                if (delimiter.length() == 1 && value.matches("[0-9]+(?:\\.[0-9]+)?")) {
                    result.append(source, start, i);continue;
                }
            } else { result.append(source.charAt(i++));continue; }
            for (int j = start; j < i; j++) result.append(source.charAt(j) == '\n' ? '\n' : ' ');
        }
        return result.toString();
    }

    Source source(CompilationUnitTree unit) {
        return sources.get(Path.of(unit.getSourceFile().toUri()).toAbsolutePath().normalize());
    }
    long start(CompilationUnitTree unit, Tree node) { return trees.getSourcePositions().getStartPosition(unit, node); }
    long end(CompilationUnitTree unit, Tree node) { return trees.getSourcePositions().getEndPosition(unit, node); }
    int line(CompilationUnitTree unit, Tree node) { return (int)Math.max(1, unit.getLineMap().getLineNumber(start(unit, node))); }
    int column(CompilationUnitTree unit, Tree node) { return (int)Math.max(1, unit.getLineMap().getColumnNumber(start(unit, node))); }
    boolean explicit(CompilationUnitTree unit, Tree node) { return start(unit, node) >= 0 && end(unit, node) >= start(unit, node); }
    boolean erroneous(CompilationUnitTree unit, Tree node) {
        return errors.intersects(unit, start(unit, node), end(unit, node));
    }
    String classId(TypeElement owner) {
        String known = classIds.get(owner);
        return known != null ? known : owner.getQualifiedName().toString();
    }
    String signature(ExecutableElement method) {
        StringJoiner parameters = new StringJoiner(",");
        // Erasure distinguishes overloaded type variables with different bounds
        // (e.g. <T extends Number> from <T extends CharSequence>) and gives a
        // stable method identity independent of each declaration's variable name.
        for (VariableElement parameter : method.getParameters()) parameters.add(task.getTypes().erasure(parameter.asType()).toString());
        return method.getSimpleName() + "(" + parameters + ")";
    }
    String methodId(ExecutableElement method) {
        return classId((TypeElement)method.getEnclosingElement()) + "#" + signature(method);
    }
    String kind(TypeElement type) {
        return type.getKind() == ElementKind.ANNOTATION_TYPE ? "annotation" : type.getKind().name().toLowerCase(Locale.ROOT);
    }
    Map<String, Object> methodComplexity(CompilationUnitTree unit, MethodTree node) {
        if (node.getBody() == null || syntaxErrors.intersects(unit, start(unit, node), end(unit, node))) return null;
        MethodMetrics metrics = new MethodMetrics();
        metrics.scan(node.getBody(), 0);
        return metrics.invalid ? null : map("cyclomatic", metrics.cyclomatic, "max_nesting", metrics.maxNesting, "definition", "ast-decisions-v1");
    }
    // Versioned AST rule: 1 + decisions, excluding nested executable scopes.
    // It deliberately does not claim parity with bytecode/vendor metrics.
    @SuppressWarnings("preview") // getLabels is reflective preview on JDK17, final on JDK21.
    static final class MethodMetrics extends TreeScanner<Void, Integer> {
        int cyclomatic = 1, maxNesting;
        boolean invalid;
        @Override public Void scan(Tree node, Integer depth) {
            if (node == null || node instanceof ClassTree || node instanceof LambdaExpressionTree) return null;
            Tree.Kind kind = node.getKind();
            if (kind == Tree.Kind.ERRONEOUS) invalid = true;
            boolean control = switch (kind) {
                case IF, WHILE_LOOP, DO_WHILE_LOOP, FOR_LOOP, ENHANCED_FOR_LOOP, CATCH, CONDITIONAL_EXPRESSION -> true;
                default -> false;
            };
            if (control || kind == Tree.Kind.CONDITIONAL_AND || kind == Tree.Kind.CONDITIONAL_OR) cyclomatic++;
            // getExpressions omits type/record patterns on modern JDKs. Labels
            // preserve every alternative, including null in "case null, default".
            if (node instanceof CaseTree branch)
                for (Tree label : branch.getLabels())
                    if (label.getKind() != Tree.Kind.DEFAULT_CASE_LABEL) cyclomatic++;
            int nested = depth + (control || kind == Tree.Kind.SWITCH || kind == Tree.Kind.SWITCH_EXPRESSION ? 1 : 0);
            maxNesting = Math.max(maxNesting, nested);
            return super.scan(node, nested);
        }
        @Override public Void visitIf(IfTree node, Integer depth) {
            scan(node.getCondition(), depth);
            scan(node.getThenStatement(), depth);
            // Else-if continues the same branch ladder; it is not a deeper body.
            scan(node.getElseStatement(), node.getElseStatement() instanceof IfTree ? depth - 1 : depth);
            return null;
        }
    }
    // getElement attributes lazily and can hit the same javac crashes as
    // analyze; the symbol then stays unresolved and its file is partial.
    javax.lang.model.element.Element element(TreePath path) {
        try {
            return trees.getElement(path);
        } catch (RuntimeException | AssertionError crash) {
            unresolvedCrashes.add(path.getCompilationUnit());
            return null;
        }
    }

    final class Declarations extends TreePathScanner<Void, Void> {
        String parentClass;
        @Override public Void visitClass(ClassTree node, Void ignored) {
            javax.lang.model.element.Element element = element(getCurrentPath());
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            Source source = source(unit);
            if (!(element instanceof TypeElement type) || source == null || !explicit(unit, node)) return null;
            String id = type.getQualifiedName().toString();
            if (id.isEmpty() || type.getNestingKind() == NestingKind.LOCAL || type.getNestingKind() == NestingKind.ANONYMOUS)
                id = (parentClass == null ? source.graphNode : parentClass) + "$"
                + (type.getSimpleName().length() == 0 ? "anonymous" : type.getSimpleName()) + "@" + line(unit, node) + ":" + column(unit, node);
            classIds.put(type, id);
            Set<Modifier> modifiers = type.getModifiers();
            String visibility = modifiers.contains(Modifier.PUBLIC) ? "public"
                : modifiers.contains(Modifier.PROTECTED) ? "protected"
                : modifiers.contains(Modifier.PRIVATE) ? "private" : "package";
            List<String> supertypes = new ArrayList<>();
            for (var parent : task.getTypes().directSupertypes(type.asType()))
                supertypes.add(task.getTypes().erasure(parent).toString());
            Collections.sort(supertypes);
            if (!type.getKind().isInterface()) {
                int depth = 0; boolean known = true;
                Set<String> visited = new HashSet<>();
                var parent = type.getSuperclass();
                String base = parent.getKind() == javax.lang.model.type.TypeKind.NONE ? null : task.getTypes().erasure(parent).toString();
                while (parent.getKind() != javax.lang.model.type.TypeKind.NONE) {
                    if (parent.getKind() == javax.lang.model.type.TypeKind.ERROR || !visited.add(parent.toString()) || depth >= 512) { known = false; break; }
                    if (!(task.getTypes().asElement(parent) instanceof TypeElement ancestor)) { known = false; break; }
                    depth++; parent = ancestor.getSuperclass();
                }
                if (node.getExtendsClause()!=null && erroneous(unit,node.getExtendsClause())) known=false;
                inheritance.add(map("class_id",id,"base_class",base,"depth",known?depth:null,"status",known?"ready":"partial"));
            }
            classes.add(map("id", id, "name", id, "package", task.getElements().getPackageOf(type).getQualifiedName().toString().replace('.', '/'),
                "file", source.file, "graph_node", source.graphNode, "line", line(unit, node), "kind", kind(type),
                "is_abstract", modifiers.contains(Modifier.ABSTRACT) || type.getKind().isInterface(),
                "visibility", visibility, "supertypes", supertypes, "language", "java"));
            String previous = parentClass;
            parentClass = id;
            super.visitClass(node, ignored);
            parentClass = previous;
            return null;
        }
        @Override public Void visitMethod(MethodTree node, Void ignored) {
            javax.lang.model.element.Element element = element(getCurrentPath());
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            Source source = source(unit);
            if (!(element instanceof ExecutableElement method) || source == null || !explicit(unit, node)) return null;
            String owner = classId((TypeElement)method.getEnclosingElement());
            String id = methodId(method);
            methodIds.put(method, id);
            Map<String, Object> declaration = map("id", id, "class_id", owner, "name", method.getSimpleName().toString(), "signature", signature(method),
                "file", source.file, "line", line(unit, node), "is_static", method.getModifiers().contains(Modifier.STATIC));
            long end = trees.getSourcePositions().getEndPosition(unit, node);
            if (end > start(unit, node)) declaration.put("end_line", (int)unit.getLineMap().getLineNumber(end - 1));
            Map<String, Object> complexity = methodComplexity(unit, node);
            if (complexity != null) {
                declaration.put("complexity", complexity);
                SourceMetrics measured = new SourceMetrics(unit);
                measured.scan(node.getBody(),0);
                methodMetrics.add(measured.value(id));
            }
            methods.add(declaration);
            return super.visitMethod(node, ignored);
        }
    }

    // AST-based Halstead vocabulary: expression/statement kinds are operators,
    // identifier and literal spellings are operands. Blocks and parentheses
    // are transparent. Nested executable scopes are measured separately.
    final class SourceMetrics extends TreeScanner<Void,Integer> {
        final CompilationUnitTree unit;
        final Set<String> operators = new HashSet<>(), operands = new HashSet<>();
        final Set<Long> lines = new HashSet<>();
        int n1,n2,cognitive;
        SourceMetrics(CompilationUnitTree unit) { this.unit=unit; }
        Map<String,Object> value(String id) { return map("method_id",id,"definition","source-maintainability/v1","cognitive",cognitive,"distinct_operators",operators.size(),"distinct_operands",operands.size(),"operators",n1,"operands",n2,"source_lines",lines.size()); }
        boolean logical(Tree node) { return node!=null && (node.getKind()==Tree.Kind.CONDITIONAL_AND || node.getKind()==Tree.Kind.CONDITIONAL_OR); }
        void tokens(Tree node) {
            if (node instanceof IdentifierTree id) { operands.add(id.getName().toString()); n2++; }
            else if (node instanceof LiteralTree) { operands.add(node.toString()); n2++; }
            else if ((node instanceof ExpressionTree || node instanceof StatementTree || node instanceof CatchTree)
                && !(node instanceof BlockTree || node instanceof ParenthesizedTree)) { operators.add(node.getKind().name()); n1++; }
            else return;
            long pos=start(unit,node); if(pos>=0) lines.add(unit.getLineMap().getLineNumber(pos));
        }
        Tree unwrap(Tree node) { while(node instanceof ParenthesizedTree p)node=p.getExpression();return node; }
        void booleanSequence(Tree node, int depth, List<Tree.Kind> sequence) {
            node=unwrap(node);
            if(logical(node)) { tokens(node); BinaryTree binary=(BinaryTree)node; booleanSequence(binary.getLeftOperand(),depth,sequence);sequence.add(node.getKind());booleanSequence(binary.getRightOperand(),depth,sequence); }
            else scan(node,depth);
        }
        void branch(IfTree node,int depth,boolean ladder) {
            tokens(node);cognitive+=ladder?1:1+depth;
            scan(node.getCondition(),depth);scan(node.getThenStatement(),depth+1);
            if(node.getElseStatement() instanceof IfTree next)branch(next,depth,true);
            else if(node.getElseStatement()!=null){cognitive++;scan(node.getElseStatement(),depth+1);}
        }
        @Override public Void scan(Tree node,Integer depth) {
            if(node==null || node instanceof ClassTree || node instanceof LambdaExpressionTree)return null;
            if(node instanceof IfTree branch){branch(branch,depth,false);return null;}
            if(logical(node)) {
                List<Tree.Kind> sequence=new ArrayList<>();booleanSequence(node,depth,sequence);Tree.Kind previous=null;
                for(Tree.Kind op:sequence){if(op!=previous)cognitive++;previous=op;}return null;
            }
            tokens(node);
            boolean nested=switch(node.getKind()){case WHILE_LOOP,DO_WHILE_LOOP,FOR_LOOP,ENHANCED_FOR_LOOP,CATCH,CONDITIONAL_EXPRESSION,SWITCH,SWITCH_EXPRESSION -> true;default -> false;};
            if(nested)cognitive+=1+depth;
            if(node instanceof BreakTree b && b.getLabel()!=null || node instanceof ContinueTree c && c.getLabel()!=null)cognitive++;
            return super.scan(node,depth+(nested?1:0));
        }
    }

    // Collect declaration types independently of calls: a signature or field
    // can create an architectural dependency even if it is never invoked.
    final class Details extends TreePathScanner<Void, Void> {
        String owner, method;
        Map<String, Object> state;
        @Override public Void visitClass(ClassTree node, Void unused) {
            String oldOwner=owner, oldMethod=method; Map<String,Object> oldState=state;
            owner=classIds.get(element(getCurrentPath())); method=null;
            if(owner!=null) {
                state=map("class_id",owner,"status",erroneous(getCurrentPath().getCompilationUnit(),node)?"partial":"ready","fields",new ArrayList<Map<String,Object>>(),"accesses",new ArrayList<Map<String,Object>>());
                classStates.put(owner,state);
                super.visitClass(node,unused);
            }
            owner=oldOwner;method=oldMethod;state=oldState;return null;
        }
        @Override public Void visitMethod(MethodTree node, Void unused) {
            String old=method;method=methodIds.get(element(getCurrentPath()));
            types(node.getReturnType(),"return");
            for(VariableTree parameter:node.getParameters()) types(parameter.getType(),"parameter");
            super.visitMethod(node,unused);method=old;return null;
        }
        @SuppressWarnings("unchecked")
        @Override public Void visitVariable(VariableTree node, Void unused) {
            var element=element(getCurrentPath());
            if(owner!=null && element instanceof VariableElement field && field.getKind().isField()) {
                String id=owner+"#field:"+field.getSimpleName();fieldIds.put(field,id);
                ((List<Map<String,Object>>)state.get("fields")).add(map("id",id,"name",field.getSimpleName().toString(),"is_static",field.getModifiers().contains(Modifier.STATIC),"file",source(getCurrentPath().getCompilationUnit()).file,"line",line(getCurrentPath().getCompilationUnit(),node)));
                types(node.getType(),"field");
            } else if(element!=null && element.getKind()!=ElementKind.PARAMETER) types(node.getType(),"local");
            return super.visitVariable(node,unused);
        }
        @Override public Void visitAnnotation(AnnotationTree node, Void unused) {
            TreePath parent=getCurrentPath().getParentPath();
            String role=parent!=null && parent.getParentPath()!=null && parent.getParentPath().getLeaf() instanceof ClassTree?"class_annotation":"annotation";
            types(node.getAnnotationType(),role);return super.visitAnnotation(node,unused);
        }
        void types(Tree type, String role) {
            if(type==null||owner==null||typeUses.size()>=maxCalls)return;
            TreePath path=TreePath.getPath(getCurrentPath().getCompilationUnit(),type);
            if(path==null)return;
            new TreePathScanner<Void,Void>() {
                void capture(Tree node) {
                    var element=element(getCurrentPath());
                    var unit=getCurrentPath().getCompilationUnit();
                    if(!(element instanceof TypeElement target)||!explicit(unit,node)||erroneous(unit,node)||target.asType().getKind()==javax.lang.model.type.TypeKind.ERROR)return;
                    String id=classId(target);
                    typeUses.add(map("from_class",owner,"from_method",method,"to_class",id,"target_label",id,"role",role,"resolution",classIds.containsKey(target)?"project":"external","file",source(unit).file,"line",line(unit,node),"code",limit(node.toString(),1000)));
                }
                @Override public Void visitIdentifier(IdentifierTree node,Void unused){capture(node);return null;}
                @Override public Void visitMemberSelect(MemberSelectTree node,Void unused){capture(node);return null;}
            }.scan(path,null);
        }
    }

    final class Invocations extends TreePathScanner<Void, Void> {
        String currentClass, currentMethod;
        // A first pass counts eligible sites per source; the second records under a
        // per-file quota. Both passes share the eligibility guards below so the
        // counted numbers match what record() can emit.
        boolean counting;
        final Map<String, Integer> counted = new TreeMap<>();
        final Map<String, Integer> quota = new HashMap<>();
        long omittedCallSites;
        int truncatedFiles;
        void allocate() {
            long total = 0;
            for (int count : counted.values()) total += count;
            if (total <= maxCalls) return; // no quota: everything fits, as before
            partial = true;
            List<String> files = new ArrayList<>(counted.keySet());
            files.sort(Comparator.comparingInt((String file) -> counted.get(file)).thenComparing(file -> file));
            int n = files.size(), level = 0, i = 0;
            long remaining = maxCalls;
            while (i < n) {
                long spread = (long)(counted.get(files.get(i)) - level) * (n - i);
                if (spread <= remaining) { remaining -= spread; level = counted.get(files.get(i)); i++; }
                else break;
            }
            for (String file : files) quota.put(file, Math.min(counted.get(file), level));
            // Distribute the rounding remainder to the largest files first (ties by
            // name), then report what the budget could not carry.
            for (int k = n - 1; k >= 0 && remaining > 0; k--) {
                String file = files.get(k);
                int give = (int)Math.min(counted.get(file) - quota.get(file), remaining);
                quota.merge(file, give, Integer::sum);
                remaining -= give;
            }
            for (Map.Entry<String, Integer> entry : counted.entrySet()) {
                int kept = quota.get(entry.getKey());
                if (kept < entry.getValue()) truncatedFiles++;
                omittedCallSites += entry.getValue() - kept;
            }
        }
        @Override public Void visitClass(ClassTree node, Void ignored) {
            javax.lang.model.element.Element element = element(getCurrentPath());
            String oldClass = currentClass, oldMethod = currentMethod;
            currentClass = classIds.get(element);
            currentMethod = null;
            if (currentClass != null) super.visitClass(node, ignored);
            currentClass = oldClass; currentMethod = oldMethod;
            return null;
        }
        @Override public Void visitMethod(MethodTree node, Void ignored) {
            javax.lang.model.element.Element element = element(getCurrentPath());
            String previous = currentMethod;
            currentMethod = methodIds.get(element);
            if (currentMethod != null) super.visitMethod(node, ignored);
            currentMethod = previous;
            return null;
        }
        @Override public Void visitMethodInvocation(MethodInvocationTree node, Void ignored) {
            if (counting) count(node);
            else record(node, element(new TreePath(getCurrentPath(), node.getMethodSelect())), node.getMethodSelect().toString());
            return super.visitMethodInvocation(node, ignored);
        }
        @Override public Void visitNewClass(NewClassTree node, Void ignored) {
            if (counting) count(node);
            else record(node, element(getCurrentPath()), node.getIdentifier().toString() + " constructor");
            return super.visitNewClass(node, ignored);
        }
        void count(Tree node) {
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            Source source = source(unit);
            if (currentClass == null || source == null || !explicit(unit, node)) return;
            counted.merge(source.file, 1, Integer::sum);
        }
        @Override public Void visitIdentifier(IdentifierTree node, Void unused) {
            field(node);return super.visitIdentifier(node,unused);
        }
        @Override public Void visitMemberSelect(MemberSelectTree node, Void unused) {
            field(node);return super.visitMemberSelect(node,unused);
        }
        @SuppressWarnings("unchecked")
        void field(Tree node) {
            var unit=getCurrentPath().getCompilationUnit();
            String id=fieldIds.get(element(getCurrentPath()));
            if(currentClass==null||currentMethod==null||id==null||!explicit(unit,node)||erroneous(unit,node))return;
            var state=classStates.get(currentClass);if(state==null)return;
            var accesses=(List<Map<String,Object>>)state.get("accesses");
            if(accesses.size()>=maxCalls){state.put("status","partial");return;}
            accesses.add(map("method",currentMethod,"field",id,"file",source(unit).file,"line",line(unit,node)));
        }
        void record(Tree node, javax.lang.model.element.Element element, String label) {
            CompilationUnitTree unit = getCurrentPath().getCompilationUnit();
            Source source = source(unit);
            if (currentClass == null || source == null || !explicit(unit, node)) return;
            if (!quota.isEmpty()) {
                Integer left = quota.get(source.file);
                if (left == null || left <= 0) return; // omission is counted in allocate()
                quota.put(source.file, left - 1);
            }
            String targetClass = null, targetMethod = null, dispatch = "unknown", resolution = "unresolved";
            // javac can expose a recovery symbol even for an ambiguous overload.
            // An error at this invocation is therefore unresolved, not a guess.
            if (!erroneous(unit, node) && element instanceof ExecutableElement method && method.getEnclosingElement() instanceof TypeElement owner
                && owner.asType().getKind() != javax.lang.model.type.TypeKind.ERROR) {
                targetClass = classId(owner);
                if (!targetClass.isEmpty()) {
                    targetMethod = methodId(method);
                    label = targetMethod;
                    resolution = classIds.containsKey(owner) ? "project" : "external";
                    dispatch = method.getKind() == ElementKind.CONSTRUCTOR ? "constructor"
                        : method.getModifiers().contains(Modifier.STATIC) ? "static"
                        : owner.getKind().isInterface() ? "interface" : "instance";
                }
            }
            if (resolution.equals("unresolved")) partial = true;
            String content = sourceText.get(unit);
            String code = limit(content.substring((int)start(unit, node), (int)end(unit, node)), 1000);
            calls.add(map("from_class", currentClass, "from_method", currentMethod, "to_class", targetClass, "to_method", targetMethod,
                "target_label", limit(label, 500), "dispatch", dispatch, "resolution", resolution, "file", source.file,
                "line", line(unit, node), "column", column(unit, node), "code", code));
        }
    }

    static String limit(String value, int length) { return value.length() > length ? value.substring(0, length) + "…" : value; }
    static Map<String, Object> map(Object... pairs) {
        Map<String, Object> result = new LinkedHashMap<>();
        for (int i = 0; i < pairs.length; i += 2) result.put((String)pairs[i], pairs[i + 1]);
        return result;
    }
    static void writeJson(Object value, Appendable out) throws IOException {
        if (value == null) { out.append("null"); return; }
        if (value instanceof Number || value instanceof Boolean) { out.append(value.toString()); return; }
        if (value instanceof Map<?, ?> map) {
            out.append('{'); boolean first = true;
            for (Map.Entry<?, ?> entry : map.entrySet()) {
                if (!first) out.append(','); first = false;
                writeJson(entry.getKey(), out); out.append(':'); writeJson(entry.getValue(), out);
            }
            out.append('}'); return;
        }
        if (value instanceof Iterable<?> items) {
            out.append('['); boolean first = true;
            for (Object item : items) {
                if (!first) out.append(','); first = false;
                writeJson(item, out);
            }
            out.append(']'); return;
        }
        String text = value.toString(); int start = 0; out.append('"');
        for (int i = 0; i < text.length(); i++) {
            char c = text.charAt(i);
            if (c == '\\' || c == '"' || c < 32) {
                out.append(text, start, i);
                if (c < 32) out.append("\\u00").append("0123456789abcdef".charAt(c >> 4)).append("0123456789abcdef".charAt(c & 15));
                else out.append('\\').append(c);
                start = i + 1;
            }
        }
        // Append intact spans so UTF-16 surrogate pairs stay together.
        out.append(text, start, text.length()).append('"');
    }
}
