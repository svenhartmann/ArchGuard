#!/usr/bin/env python3
"""Record an actual, reproducible MCP exchange; this is not an assistant transcript."""
import argparse
import json
from pathlib import Path
import select
import subprocess

from capture import prepare


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--project", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    manifest = json.loads(Path(__file__).with_name("sources.json").read_text())
    project = next(item for item in manifest["projects"] if item["id"] == "commons-collections")
    if args.project.resolve().name != project["id"]:
        parser.error("--project must point to the pinned commons-collections checkout")
    prepare(project, args.project.resolve().parent)
    process = subprocess.Popen([str(args.cli.resolve()), "mcp", "--root", str(args.project.resolve())],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, text=True, bufsize=1)
    exchanges = []

    def rpc(method, params):
        request = {"jsonrpc": "2.0", "id": len(exchanges) + 1, "method": method, "params": params}
        process.stdin.write(json.dumps(request) + "\n")
        process.stdin.flush()
        if not select.select([process.stdout], [], [], 300)[0]:
            raise TimeoutError("MCP response timed out")
        response = json.loads(process.stdout.readline())
        if "error" in response or response.get("result", {}).get("isError"):
            raise ValueError(response)
        exchanges.append({"request": request, "response": response})
        return response

    try:
        rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "archguard-marketing-reproduction", "version": "1"}})
        process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        process.stdin.flush()
        rpc("tools/call", {"name": "edge_evidence", "arguments": {
            "from": "org/apache/commons/collections4/comparators", "to": "org/apache/commons/collections4/ComparatorUtils.java"}})
        rpc("tools/call", {"name": "simulate", "arguments": {"ops": [{"cut_edge": {
            "from": "org/apache/commons/collections4/comparators", "to": "org/apache/commons/collections4/ComparatorUtils.java"}}]}})
        output = {"kind": "recorded MCP JSON-RPC session, not a Claude Code conversation",
                  "project": "Apache Commons Collections",
                  "commit": subprocess.check_output(["git", "-C", str(args.project), "rev-parse", "HEAD"], text=True).strip(),
                  "cli_version": subprocess.check_output([str(args.cli), "--version"], text=True).strip(),
                  "exchanges": exchanges}
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(json.dumps(output, indent=2) + "\n")
    finally:
        process.terminate()
        process.communicate(timeout=10)


if __name__ == "__main__":
    main()
