#!/usr/bin/env python3
"""Real HTTP regression: graph analysis must not block the live viewer.

Uses a temporary Git wrapper as a deterministic analysis barrier. No browser,
network service, source checkout modification or compiler timing assumption.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import os
from pathlib import Path
import select
import shutil
import subprocess
import sys
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", nargs="?", type=Path, default=Path("target/debug/archguard"))
    args = parser.parse_args()
    binary = args.binary.resolve()
    real_git = shutil.which("git")
    assert real_git, "Git is required for the deterministic analysis barrier"
    with tempfile.TemporaryDirectory(prefix="archguard-serve-async-") as directory:
        base = Path(directory)
        root, tools = base / "project", base / "bin"
        root.mkdir()
        tools.mkdir()
        (root / "A.ts").write_text("export const a = 1;\n")
        (root / "B.ts").write_text("export const b = 2;\n")
        (root / "archguard.toml").write_text("derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n")
        gate, entered = base / "block-analysis", base / "entered"
        gate.touch()
        wrapper = tools / "git"
        wrapper.write_text(
            f"#!{sys.executable}\n"
            "import os,sys,time\nfrom pathlib import Path\n"
            f"gate=Path({str(gate)!r})\nentered=Path({str(entered)!r})\n"
            "if gate.exists():\n"
            "    with entered.open('a') as output: output.write('entered\\n')\n"
            "    deadline=time.monotonic()+30\n"
            "    while gate.exists() and time.monotonic()<deadline: time.sleep(.01)\n"
            f"os.execv({real_git!r},[{real_git!r},*sys.argv[1:]])\n"
        )
        wrapper.chmod(0o755)
        environment = {**os.environ, "PATH": str(tools) + os.pathsep + os.environ.get("PATH", "")}
        process = subprocess.Popen(
            [str(binary), "serve", str(root), "--port", "0"],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=environment,
        )

        def request(path, timeout=2, method="GET", headers=None, sent=None):
            connection = http.client.HTTPConnection(address, timeout=timeout)
            try:
                started = time.monotonic()
                connection.request(method, path, headers=headers or {})
                if sent is not None:
                    sent.set()
                response = connection.getresponse()
                body = response.read().decode()
                elapsed = time.monotonic() - started
                assert elapsed < timeout, (path, elapsed)
                return response.status, body
            except TimeoutError as error:
                raise AssertionError(f"{path} blocked on source analysis instead of responding immediately") from error
            finally:
                connection.close()

        def await_barrier():
            deadline = time.monotonic() + 10
            while not entered.exists() and time.monotonic() < deadline:
                time.sleep(.01)
            assert entered.exists(), "the analysis worker did not reach the controlled Git barrier"

        def payload(html):
            marker = '<script id="archguard-data" type="application/json">'
            return json.loads(html.split(marker, 1)[1].split("</script>", 1)[0])

        def version():
            status, body = request("/__archguard/version")
            assert status == 200, (status, body)
            return json.loads(body)

        def wait_ready(expected):
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                state = version()
                if state["status"] == "ready" and not state["refreshing"]:
                    assert state["version"] == expected, state
                    return state
                time.sleep(.02)
            raise AssertionError("the completed analysis never became ready")

        try:
            assert select.select([process.stdout], [], [], 10)[0], "server startup timeout"
            url = process.stdout.readline().strip()
            assert url.startswith("http://127.0.0.1:"), url
            address = url.removeprefix("http://").removesuffix("/")
            status, html = request("/")
            assert status == 202, (status, html[:200])
            assert "/__archguard/version" in html, "cold shell must keep checking its analysis"
            await_barrier()
            state = version()
            assert state["refreshing"] and state["status"] == "loading", state
            assert state["version"] == "", state

            # Two explicit automation requests may wait; additional blocking
            # clients must be rejected so browser routes retain HTTP workers.
            with ThreadPoolExecutor(max_workers=2) as executor:
                sent = [threading.Event(), threading.Event()]
                checks = [executor.submit(request, "/__archguard/check", 15, sent=event) for event in sent]
                try:
                    assert all(event.wait(2) for event in sent), "check requests were not sent"
                    # Give the already-sent requests time to enter the server's
                    # wait path; the analysis barrier stays closed throughout.
                    time.sleep(.05)
                    status, _ = request("/__archguard/check")
                    assert status == 503, "extra synchronous checks must fail fast instead of starving graph/status requests"
                    for _ in range(5):
                        assert version()["status"] == "loading"
                        assert request("/")[0] == 202
                    assert request("/A.ts")[0] == 404
                    assert request("/?project=..%2Foutside")[0] == 400
                    assert request("/?project=missing")[0] == 404
                    assert request("/__archguard/check", method="POST")[0] == 405
                    assert request("/__archguard/version", headers={"Host": "untrusted.example"})[0] == 403
                    assert all(not checked.done() for checked in checks), "explicit checks must wait for completed analyses"
                    assert entered.read_text().splitlines() == ["entered"], "repeated probes must coalesce into one analysis"
                finally:
                    gate.unlink(missing_ok=True)
                completed = [checked.result(timeout=15) for checked in checks]
                assert completed[0] == completed[1], "unchanged checks keep the same revision"
                status, body = completed[0]
            first = json.loads(body)
            assert status == 200 and first["version"], (status, first)
            assert set(first) == {"version"}, "fresh-check result must retain stable revision semantics"
            wait_ready(first["version"])
            status, ready_html = request("/")
            assert status == 200 and payload(ready_html)["edges"] == [], "completed cold analysis must publish its graph"

            # The next analysis sees a new edge, but its partially computed
            # state must never leak into the previously published HTML/version.
            (root / "A.ts").write_text("import { b } from './B';\nexport const a = b;\n")
            entered.unlink()
            gate.touch()
            with ThreadPoolExecutor(max_workers=1) as executor:
                checked = executor.submit(request, "/__archguard/check", 15)
                try:
                    await_barrier()
                    state = version()
                    assert state["version"] == first["version"] and state["refreshing"], state
                    assert state["status"] == "refreshing", state
                    status, cached = request("/")
                    assert status == 200 and cached == ready_html, "warm HTML must stay immediately available and immutable"
                    for _ in range(3):
                        assert version()["version"] == first["version"]
                    assert entered.read_text().splitlines() == ["entered"], "warm refreshes must coalesce"
                    assert not checked.done()
                finally:
                    gate.unlink(missing_ok=True)
                status, body = checked.result(timeout=15)
            changed = json.loads(body)
            assert status == 200 and changed["version"] != first["version"], changed
            wait_ready(changed["version"])
            status, new_html = request("/")
            assert status == 200 and len(payload(new_html)["edges"]) == 1
            assert new_html != ready_html
            print("serve async: cold loading/status, warm cached HTML, coalesced analysis, fresh-check isolation, route guards and source invalidation passed")
        finally:
            gate.unlink(missing_ok=True)
            process.terminate()
            process.wait(timeout=5)


if __name__ == "__main__":
    main()
