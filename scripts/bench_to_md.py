#!/usr/bin/env python3
"""Render Criterion SESAME benchmark output as the Markdown tables used in
docs/benchmarks.md and cited by the SCTE 130-9 paper (§9.2).

Usage:
    python3 scripts/bench_to_md.py [CRITERION_DIR]

CRITERION_DIR defaults to $CARGO_TARGET_DIR/criterion, else ./target/criterion.

p50 = Criterion median point estimate.
p99 = 99th percentile of the per-sample iteration means (sample.json). This is a
methodology-consistent stand-in for the per-request tail, not a production SLA.
"""
import json
import os
import sys

SIZES = [1024, 4096, 16384]

PER_OP = [
    ("tier1_sign", "Tier 1 — HMAC sign"),
    ("tier1_verify", "Tier 1 — verify (body hash + HMAC)"),
    ("tier2_authz", "Tier 2 — scope authorization"),
    ("tier3_seal", "Tier 3 — AES-256-GCM seal"),
    ("tier3_open", "Tier 3 — AES-256-GCM open"),
]
COMBINED = [
    ("combined_verify_open", "Inbound — verify + authz + decrypt (Tier 1+2+3)"),
    ("combined_seal_sign", "Outbound — encrypt + sign (Tier 1+3)"),
]


def criterion_dir():
    if len(sys.argv) > 1:
        return sys.argv[1]
    base = os.environ.get("CARGO_TARGET_DIR", "target")
    return os.path.join(base, "criterion")


def pctile(xs, p):
    xs = sorted(xs)
    if not xs:
        return None
    k = (len(xs) - 1) * p
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    return xs[f] + (xs[c] - xs[f]) * (k - f)


def load(crit, group, size):
    base = os.path.join(crit, group, str(size), "new")
    est = json.load(open(os.path.join(base, "estimates.json")))
    p50 = est["median"]["point_estimate"]  # ns
    samp = json.load(open(os.path.join(base, "sample.json")))
    per = [t / i for t, i in zip(samp["times"], samp["iters"])]
    return p50, pctile(per, 0.99)


def fmt(ns):
    if ns is None:
        return "—"
    ms = ns / 1e6
    if ms >= 0.01:
        return f"{ms:.3f} ms"
    return f"{ns / 1e3:.2f} µs"


def table(crit, rows):
    out = ["| Operation | 1 KB p50 | 1 KB p99 | 4 KB p50 | 4 KB p99 | 16 KB p50 | 16 KB p99 |",
           "|---|---|---|---|---|---|---|"]
    for group, label in rows:
        cells = [label]
        for s in SIZES:
            try:
                p50, p99 = load(crit, group, s)
            except Exception:
                p50 = p99 = None
            cells += [fmt(p50), fmt(p99)]
        out.append("| " + " | ".join(cells) + " |")
    return "\n".join(out)


def main():
    crit = criterion_dir()
    if not os.path.isdir(crit):
        sys.exit(f"criterion dir not found: {crit} (run `cargo bench --bench sesame_overhead` first)")
    print("### Per-operation overhead\n")
    print(table(crit, PER_OP))
    print("\n### Combined per-request paths\n")
    print(table(crit, COMBINED))


if __name__ == "__main__":
    main()
