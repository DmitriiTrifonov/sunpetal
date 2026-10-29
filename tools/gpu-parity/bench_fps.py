#!/usr/bin/env python3
"""Frames per second of the headless runtime, software 3D against the GPU one (этап 8.3 of
docs/GPU-BACKEND.md). The same scripted input runs in every configuration with every frame drawn
(`--draw-all`), so the difference is the 3D (and the composing around it). Needs the ROM:

    SUNPETAL_ROM=... python3 tools/gpu-parity/bench_fps.py [--vblanks N] [--scales 1,2,3,4] [--msaa 0,4]

Each configuration runs REPEAT times and the best is taken (the machine's noise only adds time).
"""
import argparse, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
REPEAT = int(os.environ.get("REPEAT", "2"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--vblanks", type=int, default=3000)
    ap.add_argument("--scales", default="1,2,3,4")
    ap.add_argument("--msaa", default="0")
    ap.add_argument("--input", default=os.path.join(ROOT, "build/gpu-parity/blind.input"))
    ap.add_argument("--renderers", default="software,gpu")
    a = ap.parse_args()
    rom = os.environ.get("SUNPETAL_ROM")
    if not rom or not os.path.isfile(rom):
        sys.exit("set SUNPETAL_ROM to the ROM file")
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "sunpetal-runtime", "--features", "gpu", "--bin", "sunpetal-headless"], cwd=ROOT, check=True)
    exe = os.path.join(ROOT, "target/release/sunpetal-headless")
    with open(a.input) as f:
        script = f.read().strip()  # `--input` takes the script itself, not a file
    out = os.path.join(ROOT, "build/bench-fps")
    os.makedirs(out, exist_ok=True)
    rends = a.renderers.split(",")

    def run(extra):
        best = None
        for _ in range(REPEAT):
            t = time.time()
            p = subprocess.run([exe, rom, "--vblanks", str(a.vblanks), "--input", script, "--out", out] + extra, capture_output=True, text=True)
            dt = time.time() - t
            if p.returncode != 0 or "GPU renderer failed" in p.stderr + p.stdout:
                return None
            best = dt if best is None else min(best, dt)
        return best

    base = run([])  # nothing drawn: the game's own work
    base_ms = base / a.vblanks * 1000
    print(f"{a.vblanks} frames (best of {REPEAT}); the game alone with nothing drawn: {a.vblanks / base:.0f} fps, {base_ms:.2f} ms a frame")
    print("fps in all, and in brackets ms a frame the drawing adds over the game alone")
    print("scale msaa " + " ".join(f"{r:>17}" for r in rends))
    for msaa in a.msaa.split(","):
        for scale in a.scales.split(","):
            row = []
            for r in rends:
                t = run(["--draw-all", "--scale", scale, "--msaa", msaa, "--renderer", r])
                row.append("failed" if t is None else f"{a.vblanks / t:.0f} ({t / a.vblanks * 1000 - base_ms:+.2f})")
            print(f"{scale:>5} {msaa:>4} " + " ".join(f"{x:>17}" for x in row), flush=True)


main()
