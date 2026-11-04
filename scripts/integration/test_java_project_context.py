#!/usr/bin/env python3
"""Compiler-backed project-context regression; JDK 17+, no project build."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

HELPER = Path(__file__).resolve().parents[2] / "crates/archguard-engine/src/ArchguardCallScanner.java"
JAVAC_API = "jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED"
ENV = {k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")}


def main():
    with tempfile.TemporaryDirectory(prefix="archguard-java-context-") as temp:
        root = Path(temp)
        empty = root / "empty"
        empty.mkdir()
        source = root / "Client.java"
        source.write_text('class Client { void run() { dep.Api.send("value"); } }')
        manifest = root / "sources"
        manifest.write_text("\0".join((str(source), "Client.java", "Client.java")) + "\0")
        settings = root / "context"

        def analyze(release="auto", classpath=(), descriptors=()):
            settings.write_text("\0".join((str(release), os.pathsep.join(map(str, classpath)), *map(str, descriptors))) + "\0")
            result = subprocess.run(["java", "-Xmx512m", "--add-exports", JAVAC_API, "--class-path", str(empty), str(HELPER), str(manifest), str(empty), str(settings)], capture_output=True, text=True, env=ENV, timeout=45, check=True)
            assert not (root / "Client.class").exists(), "analysis must not generate project classes"
            return json.loads(result.stdout)

        missing = analyze()
        assert missing["status"] == "partial"
        assert missing["calls"][0]["resolution"] == "unresolved"
        dependency = root / "dep/Api.java"
        dependency.parent.mkdir()
        dependency.write_text('package dep; public class Api { public static void send(String value) {} }')
        classes = root / "dependency-classes"
        classes.mkdir()
        subprocess.run(["javac", "-d", str(classes), str(dependency)], env=ENV, check=True, capture_output=True)
        jar = root / "api.jar"
        subprocess.run(["jar", "cf", str(jar), "-C", str(classes), "."], env=ENV, check=True, capture_output=True)
        resolved = analyze(classpath=[jar])
        assert resolved["status"] == "ready", resolved["diagnostics"]
        assert resolved["calls"][0]["to_method"] == "dep.Api#send(java.lang.String)"
        assert resolved["calls"][0]["resolution"] == "external"
        assert resolved["analyzers"][0]["backend"] == "javac"
        assert resolved["classes"][0]["language"] == "java"

        source.write_text('class Client { String run() { return "text".strip(); } }')
        pom = root / "pom.xml"
        pom.write_text('<project><properties><java.level>8</java.level><maven.compiler.release>${java.level}</maven.compiler.release></properties></project>')
        legacy = analyze(descriptors=[pom])
        assert legacy["java_release"] == 8
        assert legacy["status"] == "partial", "Java 8 context cannot resolve the Java 11 strip API"
        override = analyze(release=17, descriptors=[pom])
        assert override["java_release"] == 17 and override["status"] == "ready", override["diagnostics"]
        assert override["calls"][0]["to_method"] == "java.lang.String#strip()"
        pom.write_text('<project><build><pluginManagement><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><configuration><release>8</release></configuration></plugin></plugins></pluginManagement></build></project>')
        managed = analyze(descriptors=[pom])
        assert managed["java_release"] == 8, "Maven compiler pluginManagement config applies to the build lifecycle"
        assert managed["status"] == "partial", "managed Java 8 API context cannot resolve String.strip"
        pom.write_text('<project><build><pluginManagement><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><configuration><release>8</release></configuration></plugin></plugins></pluginManagement><plugins><plugin><artifactId>maven-compiler-plugin</artifactId><configuration><release>17</release></configuration></plugin></plugins></build></project>')
        managed_override = analyze(descriptors=[pom])
        assert managed_override["java_release"] == 17 and managed_override["status"] == "ready", "direct plugin config overrides pluginManagement"
        pom.write_text('<project><properties><maven.compiler.release>17</maven.compiler.release></properties></project>')
        assert analyze(descriptors=[pom])["status"] == "ready"
        gradle = root / "build.gradle.kts"
        gradle.write_text('java { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }')
        assert analyze(descriptors=[gradle])["java_release"] == 17
        gradle.write_text('// JavaLanguageVersion.of(8)\njava { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }')
        assert analyze(descriptors=[gradle])["java_release"] == 17, "commented config is not a language setting"
        gradle.write_text('val example = "languageVersion.set(JavaLanguageVersion.of(8))"\njava { toolchain { languageVersion.set(JavaLanguageVersion.of(17)) } }')
        assert analyze(descriptors=[gradle])["java_release"] == 17, "string content cannot select the language level"
        gradle.write_text('java { toolchain { languageVersion.set(JavaLanguageVersion.of(21)) } }\ntasks.withType<JavaCompile> { options.release.set(17) }')
        assert analyze(descriptors=[gradle])["java_release"] == 17, "API release overrides toolchain JVM version"
        default = analyze()
        assert default["java_release"] >= 17
        assert default["analyzers"][0]["version"], "actual runtime/compiler context must be visible"
        pom.write_text('<!DOCTYPE x [<!ENTITY level SYSTEM "file:///etc/passwd">]><project><properties><maven.compiler.release>&level;</maven.compiler.release></properties></project>')
        unsafe = analyze(descriptors=[pom])
        assert unsafe["status"] == "partial"
        assert any("Maven" in d for d in unsafe["diagnostics"])
        assert not any("root:" in d for d in unsafe["diagnostics"]), "external XML entities are not read"

        # A global compilation can resolve this call, although the two modules
        # have no declared dependency. Coverage must disclose that boundary gap.
        left = root / "left"
        right = root / "right"
        left.mkdir(); right.mkdir()
        a = left / "A.java"; b = right / "B.java"
        a.write_text('package a; public class A { void run() { b.B.ping(); } }')
        b.write_text('package b; public class B { public static void ping() {} }')
        for directory in (left, right):
            (directory / "pom.xml").write_text('<project><properties><maven.compiler.release>17</maven.compiler.release></properties></project>')
        manifest.write_text("\0".join((str(a),"left/A.java","a/A.java",str(b),"right/B.java","b/B.java")) + "\0")
        separate = analyze(descriptors=[left / "pom.xml", right / "pom.xml"])
        assert separate["status"] == "partial", "one compilation does not prove independent module dependency visibility"
        assert any("module" in diagnostic and "boundar" in diagnostic for diagnostic in separate["diagnostics"]), separate["diagnostics"]
        assert any(call["to_method"] == "b.B#ping()" for call in separate["calls"]), "retain observed compiler evidence, explicitly scoped as partial"
        print("Java context: external JAR resolution, Maven/Gradle release, config precedence, runtime fallback, XML boundary and no project bytecode passed")


if __name__ == "__main__":
    main()
