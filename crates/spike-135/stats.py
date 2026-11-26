# usage: stats.py LOG...
# Frame times per phase ("## name" lines; a log without them is one phase).
# The interval counts only frames less than 200 ms apart (continuous motion).
import re, sys

def pct(values, q):
    values = sorted(values)
    return values[round(q * (len(values) - 1))]

for path in sys.argv[1:]:
    phases, name, last = {}, "all", None
    wheel = {}
    for line in open(path):
        if line.startswith("## "):
            name = line[3:].strip()
        elif line.startswith("frame="):
            last = dict(re.findall(r"(\w+)=(\S+)", line))
            phases.setdefault(name, []).append(last)
        elif line.startswith("paint_us=") and last:
            last["paint_us"] = line.split("=")[1]
        elif line.startswith("wheel "):
            kind, value = line.split()[1:3]
            wheel.setdefault(kind, []).append(abs(float(value)))
    for name, rows in phases.items():
        if len(rows) < 20:
            continue
        cpu = [(int(r["prepaint_us"]) + int(r.get("paint_us", 0))) / 1000 for r in rows]
        gaps = [i / 1000 for i in (int(r["interval_us"]) for r in rows[1:]) if 0 < i < 200_000]
        extra = [int(r["remeasure_us"]) / 1000 for r in rows]
        print(f"{name}: frames={len(rows)}")
        print(f"  element CPU ms   p50 {pct(cpu, .5):.3f}  p99 {pct(cpu, .99):.3f}  max {max(cpu):.3f}")
        print(f"  remeasure ms     p50 {pct(extra, .5):.3f}  p99 {pct(extra, .99):.3f}")
        if gaps:
            late = sum(g > 25 for g in gaps)
            print(f"  frame interval ms p50 {pct(gaps, .5):.2f}  p99 {pct(gaps, .99):.2f}  max {max(gaps):.2f}  over 25 ms: {late} of {len(gaps)}")
    for kind, values in wheel.items():
        print(f"wheel events ({kind}): {len(values)}, |delta| p50 {pct(values, .5):.3f} max {max(values):.3f}")
