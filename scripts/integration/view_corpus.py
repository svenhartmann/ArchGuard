#!/usr/bin/env python3
"""Golden-corpus harness for the ui-flow and domain views (stdlib only).

Runs `archguard ui-flow` and `archguard domain` (--format json) on every top-level
directory of a read-only corpus, stores raw outputs under target/view-corpus/<label>/
and compares two labelled runs, so "byte-identical on the corpus" is one command.

  view_corpus.py run --bin target/release/archguard --label before [--apps a,b] [--corpus DIR] [--jobs N] [--timeout 600]
  view_corpus.py compare before after [--expect-changed a,b]   # exit 0 iff all other apps are identical
  view_corpus.py access-diff DIR_BASE DIR_HEAD                  # exit 1 iff an element moved towards Open
  view_corpus.py table after                                  # markdown table of summary.json

Files per app and view: <app>.<view>.json (stdout), .err (stderr), .meta.json (exit code, seconds,
timeout). The corpus is only read, never written.
"""
import argparse
import collections
import concurrent.futures as cf
import hashlib
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "target" / "view-corpus"
CORPUS = Path("/Users/svenhartmann/Projects/000_HSE/soft-arch/examples/checkouts")
VIEWS = ("ui-flow", "domain")


def ui_counts(d):
    f, s = d["flow"], d["summary"]
    diag = f.get("diagnostics", [])
    return {
        "screens": s["screens"], "actions": s["actions"], "unresolved_actions": s["actions_unresolved"],
        "endpoints_protected": s["endpoints_by_access"].get("protected", 0),
        "endpoints_open": s["endpoints_by_access"].get("open", 0),
        "endpoints_undeclared": s["endpoints_by_access"].get("undeclared", 0),
        "diagnostics": len(diag), "gaps": sum(1 for x in diag if x.get("severity") == "gap"),
        "findings": len(d.get("findings", [])), "complete": f["complete"],
    }


def domain_counts(d):
    m = d["model"]
    sms = m.get("state_machines", [])
    n = lambda k: len(m.get(k, []))  # empty models omit their lists
    return {
        "entities": n("entities"), "rules": n("rules"), "events": n("events"),
        "state_machines": len(sms), "state_machines_with_transitions": sum(1 for x in sms if x.get("transitions")),
        "integration_events": n("integration_events"), "use_cases": n("use_cases"),
        "validations": n("validations"), "diagnostics": len(m.get("diagnostics", [])),
        "complete": m.get("complete"),
    }


COUNTERS = {"ui-flow": ui_counts, "domain": domain_counts}


def run_one(binary, label, app, view, corpus, timeout):
    out = OUT / label
    base = f"{app}.{view}"
    start = time.monotonic()
    meta = {"exit": None, "seconds": None, "timeout": False}
    try:
        # own session so a timeout can kill helpers (javac, dotnet) too
        p = subprocess.Popen([binary, view, str(corpus / app), "--format", "json"],
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = p.communicate(timeout=timeout)
            meta["exit"] = p.returncode
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGKILL)
            meta["timeout"] = True
            stdout, stderr = p.communicate()
    except OSError as e:  # binary missing or not executable
        stdout, stderr = b"", str(e).encode()
    meta["seconds"] = round(time.monotonic() - start, 2)
    (out / f"{base}.json").write_bytes(stdout)
    (out / f"{base}.err").write_bytes(stderr)
    (out / f"{base}.meta.json").write_text(json.dumps(meta))
    rec = dict(meta, sha256=hashlib.sha256(stdout).hexdigest())
    try:
        rec["counts"] = COUNTERS[view](json.loads(stdout))
    except Exception as e:  # timeout, crash or changed schema: keep the record, no counts
        rec["counts"] = None
        rec["count_error"] = f"{type(e).__name__}: {e}"
    return app, view, rec


def safe(parser, what, name):
    if not name or "/" in name or ".." in name:
        parser.error(f"{what} must not be empty or contain '/' or '..': {name!r}")


def cmd_run(a):
    for name in [a.label] + (a.apps.split(",") if a.apps else []):
        safe(a.parser, "--label/--apps value", name)
    corpus = Path(a.corpus)
    apps = a.apps.split(",") if a.apps else sorted(p.name for p in corpus.iterdir() if p.is_dir() and not p.name.startswith("."))
    (OUT / a.label).mkdir(parents=True, exist_ok=True)
    summary = {app: {} for app in apps}
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        futs = [ex.submit(run_one, str(Path(a.bin).resolve()), a.label, app, v, corpus, a.timeout)
                for app in apps for v in VIEWS]
        for f in cf.as_completed(futs):
            app, view, rec = f.result()
            summary[app][view] = rec
            print(f"{app} {view}: exit={rec['exit']} {rec['seconds']}s" + (" TIMEOUT" if rec["timeout"] else ""),
                  flush=True)
    (OUT / a.label / "summary.json").write_text(json.dumps(summary, indent=1, sort_keys=True))


def load(label):
    return json.loads((OUT / label / "summary.json").read_text())


def cmd_compare(a):
    A, B = load(a.a), load(a.b)
    expected = set(a.expect_changed.split(",")) if a.expect_changed else set()
    bad = 0
    ok = lambda r: r and r["exit"] == 0 and not r["timeout"]
    for app in sorted(set(A) | set(B)):
        if app not in A or app not in B:
            print(f"{app}: only in {a.a if app in A else a.b}" + ("" if app in expected else "  (UNEXPECTED)"))
            bad += app not in expected
            continue
        for view in VIEWS:
            x, y = A[app].get(view), B[app].get(view)
            if ok(x) and ok(y) and x["sha256"] == y["sha256"]:
                print(f"{app} {view}: IDENTICAL")
                continue
            note = "" if app in expected else "  (UNEXPECTED)"
            bad += app not in expected
            if not (ok(x) and ok(y)):
                where = "both" if not ok(x) and not ok(y) else (a.a if not ok(x) else a.b)
                print(f"{app} {view}: FAILED in {where}{note}")
                continue
            cx, cy = x.get("counts") or {}, y.get("counts") or {}
            deltas = [f"{k} {cx.get(k)}->{cy.get(k)}" for k in sorted(set(cx) | set(cy)) if cx.get(k) != cy.get(k)]
            print(f"{app} {view}: CHANGED {', '.join(deltas) or 'bytes only'}{note}")
    sys.exit(1 if bad else 0)


KINDS = ("endpoints", "screens", "links", "actions")
STATES = ("open", "undeclared", "protected", "unrecognised")
COND_KEYS = {"all", "any", "not", "role", "policy", "custom", "unknown"}  # Condition variants with data
COND_UNITS = {"authenticated", "never"}  # Condition variants without data serialize as strings


def _state(v, legacy_null):
    """Old condition or {"protected": ..} -> protected; "open"/"undeclared"; null -> legacy_null; else unrecognised."""
    if v is None:
        return legacy_null
    if isinstance(v, str) and v in ("open", "undeclared"):
        return v
    if (isinstance(v, str) and v in COND_UNITS) or (isinstance(v, dict) and len(v) == 1 and next(iter(v)) in COND_KEYS | {"protected"}):
        return "protected"
    return "unrecognised"


def _name(o):
    if isinstance(o, dict):
        k, v = next(iter(o.items()), ("", ""))
        return f"{k}:{v if isinstance(v, str) else json.dumps(v, sort_keys=True)}"
    return str(o)


def access_elements(flow):
    """{(kind, identity): Counter(state)}; elements sharing a key are all kept. An absent/null screen, link or action condition is
    undeclared in both shapes (an old null is no extra state, so old-vs-new prints no count noise)."""
    null = "undeclared"
    out = collections.defaultdict(collections.Counter)
    for e in flow.get("endpoints", []):
        out[("endpoints", f"{e.get('application') or ''} {e['method']} {e['path']}".strip())][_state(e.get("access"), "undeclared")] += 1
    for e in flow.get("screens", []):
        ident = f"{e.get('application') or ''} {e.get('route') or e['id']}".strip()
        out[("screens", ident)][_state(e.get("access"), null)] += 1
    for k, kind, name in (("transitions", "links", lambda e: f"{_name(e['from'])} -> {_name(e['target'])} [{e.get('trigger') or ''}]"),
                          ("actions", "actions", lambda e: f"{_name(e['screen'])} -> {_name(e['call'])} [{e.get('trigger') or ''}]")):
        for e in flow.get(k, []):
            out[(kind, name(e))][_state(e["access"] if "access" in e else e.get("condition"), null)] += 1
    assert sum(sum(c.values()) for c in out.values()) == sum(len(flow.get(k, [])) for k in ("endpoints", "screens", "transitions", "actions"))
    return out


def _load_flows(d):
    return {f.name[: -len(".ui-flow.json")]: json.loads(f.read_text())["flow"]
            for f in sorted(Path(d).glob("*.ui-flow.json")) if f.stat().st_size}


def access_moves(base, head):
    """(per-app count tables, moves towards Open) for two {app: flow} dicts."""
    counts, moves, bad = {}, [], []
    for app in sorted(set(base) | set(head)):
        b = access_elements(base[app]) if app in base else {}
        h = access_elements(head[app]) if app in head else {}
        counts[app] = {k: tuple({s: sum(c[s] for (kk, _), c in e.items() if kk == k) for s in STATES} for e in (b, h)) for k in KINDS}
        bad += [(app, kind, ident, c["unrecognised"]) for e in (b, h) for (kind, ident), c in sorted(e.items()) if c["unrecognised"]]
        moves += [(app, kind, ident, dict(b.get((kind, ident), {})) or "absent", dict(c)) for (kind, ident), c in sorted(h.items())
                  if c["open"] > b.get((kind, ident), {}).get("open", 0)]
    return counts, moves, sorted(set(bad))


def cmd_access_diff(a):
    counts, moves, bad = access_moves(_load_flows(a.base), _load_flows(a.head))
    for app, kinds in counts.items():
        deltas = [f"{k}:" + ",".join(f"{s[0].upper()}{h[s] - b[s]:+d}" for s in STATES if h[s] != b[s])
                  for k, (b, h) in kinds.items() if b != h]
        print(f"{app}: " + ("  ".join(deltas) or "no change"))
    for app, kind, ident, was, now in moves:
        print(f"TOWARDS OPEN {app} {kind} {ident} (base {was} -> head {now})")
    for app, kind, ident, n in bad:
        print(f"UNRECOGNISED access value {app} {kind} {ident} (x{n})")
    print(f"{len(moves)} element(s) moved towards Open, {len(bad)} unrecognised")
    sys.exit(2 if bad else 1 if moves else 0)  # 2 = cannot trust the result


def cmd_table(a):
    S = load(a.label)
    ui = ["screens", "actions", "unresolved_actions", "endpoints_protected", "endpoints_open",
          "endpoints_undeclared", "diagnostics", "gaps", "complete"]
    dm = ["entities", "rules", "events", "state_machines", "state_machines_with_transitions",
          "integration_events", "use_cases", "validations", "diagnostics", "complete"]
    for view, cols in (("ui-flow", ui), ("domain", dm)):
        print(f"\n### {view}\n\n| app | exit | s | " + " | ".join(cols) + " |")
        print("|" + "---|" * (len(cols) + 3))
        for app in sorted(S):
            r = S[app].get(view)
            if not r:
                continue
            c = r["counts"] or {}
            exit_ = "timeout" if r["timeout"] else r["exit"]
            print(f"| {app} | {exit_} | {r['seconds']} | " + " | ".join(str(c.get(k, "")) for k in cols) + " |")


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("--bin", required=True)
    r.add_argument("--label", required=True)
    r.add_argument("--apps")
    r.add_argument("--corpus", default=str(CORPUS))
    r.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    r.add_argument("--timeout", type=int, default=600)
    r.set_defaults(f=cmd_run, parser=r)
    c = sub.add_parser("compare")
    c.add_argument("a")
    c.add_argument("b")
    c.add_argument("--expect-changed")
    c.set_defaults(f=cmd_compare)
    t = sub.add_parser("table")
    t.add_argument("label")
    t.set_defaults(f=cmd_table)
    d = sub.add_parser("access-diff")
    d.add_argument("base")
    d.add_argument("head")
    d.set_defaults(f=cmd_access_diff)
    a = p.parse_args(argv)
    a.f(a)


if __name__ == "__main__":
    main()
