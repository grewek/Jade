# usage: stats134.py LOG...  -> per-log phase agreement and timing summary
import re, sys, statistics as st
for path in sys.argv[1:]:
    frames = []
    for line in open(path):
        if not line.startswith("frame="):
            continue
        f = dict(re.findall(r"(\w+)=(\[[^\]]*\]|\S+)", line))
        f["m_us"] = sum(int(x) for x in re.findall(r":(\d+)us", f["measure"]))
        frames.append(f)
    bad = [f for f in frames if not (f["prepaint_w"] == f["shaped_w"] == f["paint_w"]
           and (f["used"] == "[]" or f["used"].strip("[]").split()[-1] == f["prepaint_w"]))]
    changed = [f for i, f in enumerate(frames) if i and f["prev_w"] != f["prepaint_w"]]
    total = lambda f: (f["m_us"] + int(f["prepaint_us"]) + int(f["paint_us"])) / 1000
    widths = []
    for f in frames:
        if not widths or widths[-1] != f["prepaint_w"]:
            widths.append(f["prepaint_w"])
    stale = [f for f in frames if f["measure"] != "[]" and f["h_measure"] != f["h_after_prepaint"]]
    steady = sorted(total(f) for i, f in enumerate(frames) if i and f not in changed)
    print(f"{path.split('/')[-1]}: frames={len(frames)} disagreeing={len(bad)} width_changes={len(changed)} widths={widths}")
    print(f"  first frame: {total(frames[0]):.2f} ms")
    if changed:
        c = [total(f) for f in changed]
        print(f"  width-change frames ms: median {st.median(c):.2f} max {max(c):.2f}")
    print(f"  other frames ms: median {st.median(steady):.2f} p99 {steady[int(.99*(len(steady)-1))]:.2f} max {steady[-1]:.2f}")
    m = [f["m_us"]/1000 for f in frames if f["measure"] != "[]"]
    if m:
        print(f"  measure ms: median {st.median(m):.2f} max {max(m):.2f}")
    print(f"  frames whose height changed in prepaint: {len(stale)}, largest {max((abs(float(f['h_after_prepaint'])-float(f['h_measure'])) for f in stale), default=0):.0f} px")
