#!/usr/bin/env python3
"""Real javac declaration evidence for architecture abstraction metrics."""
from test_java_calls import analyze

result = analyze({
    "api/Port.java": "package api; public interface Port { void send(); }",
    "core/Base.java": "package core; public abstract class Base implements api.Port {}",
    "core/Adapter.java": "package core; public class Adapter extends Base { public void send() {} }",
    "core/Internal.java": "package core; class Internal { private static class Hidden {} }",
})
assert result["status"] == "ready", result["diagnostics"]
classes = {item["id"]: item for item in result["classes"]}
assert classes["api.Port"].get("is_abstract") is True, "interfaces must carry explicit abstractness evidence"
assert classes["core.Base"]["is_abstract"] is True
assert classes["core.Adapter"]["is_abstract"] is False
assert classes["core.Adapter"]["visibility"] == "public"
assert classes["core.Internal"]["visibility"] == "package"
assert classes["core.Internal.Hidden"]["visibility"] == "private"
assert classes["core.Base"]["supertypes"] == ["api.Port", "java.lang.Object"]
assert classes["core.Adapter"]["supertypes"] == ["core.Base"]
print("Java declarations: interface/abstract/concrete types, visibility and supertypes passed")
