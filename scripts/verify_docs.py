#!/usr/bin/env python3
"""Verify every ```klang sample in docs/*.md against the real binary.

Usage: scripts/verify_docs.py [--bin PATH_TO_KLANG]
Exit nonzero on any mismatch, missing directive, or UNVERIFIED marker.
"""
import os
import re
import subprocess
import sys
import tempfile

DOCS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "docs")
BIN = None

BLOCK = re.compile(r"```klang\n(.*?)```", re.DOTALL)


def parse_directive(first_line):
    t = first_line.strip()
    if t == "// @check-ok":
        return ("check-ok", None)
    m = re.match(r"// @check-fail\s+(E-[A-Z0-9-]+)\s*$", t)
    if m:
        return ("check-fail", m.group(1))
    m = re.match(r"// @run-fail\s+(E-[A-Z0-9-]+)\s*$", t)
    if m:
        return ("run-fail", m.group(1))
    m = re.match(r"// @run prints:(.*);\s*return:\s*(-?\d+)\s*$", t)
    if m:
        prints = [p.strip() for p in m.group(1).split("|")]
        return ("run", (prints, int(m.group(2))))
    return (None, None)


def run_klang(cmd, src, entry=None):
    with tempfile.NamedTemporaryFile(
        suffix=".klang", mode="w", delete=False
    ) as f:
        f.write(src)
        path = f.name
    try:
        argv = [BIN, cmd, path]
        if entry is not None:
            argv.append(entry)
        r = subprocess.run(
            argv,
            capture_output=True,
            text=True,
            timeout=120,
        )
        return r
    finally:
        os.unlink(path)


def main():
    global BIN
    BIN = None
    argv = sys.argv[1:]
    if "--bin" in argv:
        BIN = argv[argv.index("--bin") + 1]
    if BIN is None:
        BIN = os.environ.get(
            "KLANG_BIN", "/tmp/klang-target/debug/klang"
        )
    if not os.path.exists(BIN):
        print(f"klang binary not found: {BIN}")
        return 2
    files = sorted(f for f in os.listdir(DOCS) if f.endswith(".md"))
    total = 0
    failures = []
    counts = {"check-ok": 0, "run": 0, "check-fail": 0, "run-fail": 0}
    for fn in files:
        text = open(os.path.join(DOCS, fn)).read()
        if "UNVERIFIED" in text:
            failures.append(f"{fn}: contains UNVERIFIED marker")
        for i, m in enumerate(BLOCK.finditer(text)):
            total += 1
            src = m.group(1)
            first = src.split("\n", 1)[0] if src else ""
            kind, want = parse_directive(first)
            label = f"{fn} block {i + 1}"
            if kind is None:
                failures.append(f"{label}: missing/invalid directive: {first!r}")
                continue
            counts[kind] += 1
            if kind == "check-ok":
                r = run_klang("check", src)
                if "check: OK" not in r.stdout:
                    failures.append(f"{label}: expected check clean, got:\n{r.stdout}\n{r.stderr}")
            elif kind == "check-fail":
                r = run_klang("check", src)
                failed = "check: FAIL" in r.stdout or "parse: FAIL" in r.stdout
                if not failed or f'"{want}"' not in r.stdout:
                    failures.append(f"{label}: expected check-fail {want}, got:\n{r.stdout}\n{r.stderr}")
            elif kind == "run":
                prints, ret = want
                r = run_klang("run", src, "main")
                # CLI form is `run <file> [entry]`; pass entry explicitly.
                got_prints = [
                    l[len("print: "):]
                    for l in r.stdout.splitlines()
                    if l.startswith("print: ")
                ]
                mret = re.search(r"run \w+\(\) = (-?\d+)", r.stdout)
                got_ret = int(mret.group(1)) if mret else None
                if got_prints != prints or got_ret != ret:
                    failures.append(
                        f"{label}: expected prints={prints} return={ret}, "
                        f"got prints={got_prints} return={got_ret}\n{r.stdout}\n{r.stderr}"
                    )
            elif kind == "run-fail":
                r = run_klang("check", src)
                if "check: OK" not in r.stdout:
                    failures.append(f"{label}: run-fail sample must check clean, got:\n{r.stdout}")
                    continue
                r = run_klang("run", src, "main")
                if "run: FAIL" not in r.stdout or f'"{want}"' not in r.stdout:
                    failures.append(f"{label}: expected run-fail {want}, got:\n{r.stdout}\n{r.stderr}")
    print(f"verified {total} samples: {counts}")
    if failures:
        print(f"{len(failures)} FAILURES:")
        for f in failures:
            print("----\n" + f)
        return 1
    print("all docs samples verified, 0 UNVERIFIED markers")
    return 0


if __name__ == "__main__":
    sys.exit(main())
