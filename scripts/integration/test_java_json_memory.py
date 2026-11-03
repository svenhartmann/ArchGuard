"""Serialize a large repeated result with a 64 MiB JVM, verifying every output byte."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HELPER = ROOT / "crates/archguard-engine/src/ArchguardCallScanner.java"
ENV = {k: v for k, v in os.environ.items() if k not in ("JDK_JAVA_OPTIONS", "JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "CLASSPATH")}
PROBE = r'''
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
class JsonMemoryProbe {
  public static void main(String[] args) throws Exception {
    String text = "x".repeat(512) + "\n\t\"\\" + "ä 🌍";
    Object value = Collections.nCopies(200000, text);
    var out = new BufferedWriter(new OutputStreamWriter(System.out, StandardCharsets.UTF_8));
    try {
      ArchguardCallScanner.class.getDeclaredMethod("writeJson", Object.class, Appendable.class).invoke(null, value, out);
    } catch (NoSuchMethodException legacy) {
      // Exercise the pre-streaming serializer too: its actual heap exhaustion
      // is the regression, not a missing-method/compilation failure.
      out.write((String) ArchguardCallScanner.class.getDeclaredMethod("json", Object.class).invoke(null, value));
    }
    out.flush();
  }
}
'''


def main():
    with tempfile.TemporaryDirectory(prefix="archguard-json-memory-") as directory:
        temp = Path(directory)
        probe = temp / "JsonMemoryProbe.java"
        probe.write_text(PROBE)
        subprocess.run(["javac", "-encoding", "UTF-8", "-d", str(temp), str(HELPER), str(probe)], check=True, capture_output=True, env=ENV, timeout=45)
        output = temp / "result.json"
        with output.open("wb") as stream:
            result = subprocess.run(["java", "-Xmx64m", "-cp", str(temp), "JsonMemoryProbe"], stdout=stream, stderr=subprocess.PIPE, env=ENV, timeout=45)
        assert result.returncode == 0, result.stderr.decode()
        value = json.dumps("x" * 512 + '\n\t"\\' + "ä 🌍", ensure_ascii=False, separators=(",", ":")).encode()
        # The scanner deliberately uses equivalent \u escapes for controls.
        value = value.replace(b"\\n", b"\\u000a").replace(b"\\t", b"\\u0009")
        expected = hashlib.sha256(b"[")
        for index in range(200000):
            if index:
                expected.update(b",")
            expected.update(value)
        expected.update(b"]")
        actual = hashlib.sha256()
        with output.open("rb") as stream:
            for block in iter(lambda: stream.read(1024 * 1024), b""):
                actual.update(block)
        assert actual.digest() == expected.digest(), "Output differs: Unicode, escaping, ordering and entries must be preserved"
        print(f"Java JSON: {output.stat().st_size:,} verified bytes serialized within a 64 MiB heap")


if __name__ == "__main__":
    main()
