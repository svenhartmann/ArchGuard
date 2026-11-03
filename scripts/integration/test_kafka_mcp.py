"""Optional integration test against the pinned Kafka checkout; read-only MCP requests."""
import json
from pathlib import Path
import select
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
binary = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target/release/archguard").resolve()
checkout = ROOT / "examples/checkouts/kafka"
out = ROOT / ".archguard/reports/kafka-world"
with (out / "mcp.log").open("w") as log:
    process = subprocess.Popen([str(binary), "mcp", "--root", str(checkout)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, text=True, bufsize=1)
    ident = 0

    def rpc(method, params):
        global ident
        ident += 1
        process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": ident, "method": method, "params": params}) + "\n")
        process.stdin.flush()
        while True:
            assert select.select([process.stdout], [], [], 180)[0], "Kafka MCP timed out"
            line = process.stdout.readline()
            assert line, "Kafka MCP process ended; see mcp.log"
            response = json.loads(line)
            if response.get("id") == ident:
                return response

    def call(name, arguments):
        response = rpc("tools/call", {"name": name, "arguments": arguments})
        assert "error" not in response, response
        result = response["result"]
        assert not result.get("isError"), result
        text = result["content"][0]["text"]
        assert len(text.encode()) <= 4096
        return json.loads(text)

    try:
        start = time.monotonic()
        rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "kafka-integration", "version": "1"}})
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n'); process.stdin.flush()
        atlas = call("architecture_atlas", {"within": "org/apache/kafka", "limit": 3})
        assert atlas["metadata"]["cycle_groups"] > 0
        assert atlas["next_offset"] is not None
        second = call("architecture_atlas", {"within": "org/apache/kafka", "limit": 3, "offset": atlas["next_offset"], "expected_revision": atlas["metadata"]["revision"]})
        assert second["metadata"]["revision"] == atlas["metadata"]["revision"]
        world = call("architecture_world", {"query": "CommonClientConfigs", "limit": 3})
        assert world["items"] and any(row["class_id"] == "org.apache.kafka.clients.CommonClientConfigs" for row in world["items"])
        assert any(row["max_complexity"] is not None for row in world["items"]), "Large-project method measurements must survive serialization"
        findings = call("architecture_findings", {"kind": "cycle", "limit": 1})
        assert findings["items"]
        detail = call("architecture_finding", {"finding_id": findings["items"][0]["id"], "expected_revision": findings["metadata"]["revision"], "limit": 4})
        assert detail["items"]
        result = {"atlas": atlas["metadata"], "world_sample": world["items"], "cycle_finding": findings["items"][0], "detail_records": len(detail["items"]), "bounded_responses": True, "seconds": round(time.monotonic() - start, 2)}
        (out / "mcp-verification.json").write_text(json.dumps(result, indent=2) + "\n")
        print(json.dumps({"package_cycle_groups": atlas["metadata"]["cycle_groups"], "class_metrics": True, "finding_details": True, "bounded_responses": True, "seconds": result["seconds"]}))
    finally:
        process.terminate(); process.communicate(timeout=10)
