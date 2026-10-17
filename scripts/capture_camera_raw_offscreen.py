"""Capture native Camera Raw states taller than the monitor without editing live JSON."""
import argparse
import hashlib
import json
import math
import os
import struct
import subprocess
from pathlib import Path

from capture_camera_raw import STATES, includes_view, photo_identity, ordered_panels


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=Path("target/debug/examples/snapshot.exe"))
    parser.add_argument("--image", type=Path, required=True)
    parser.add_argument("--layout", type=Path, default=Path("design/camera-raw/layout.json"))
    parser.add_argument("--tokens", type=Path, default=Path("design/camera-raw/tokens.json"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--states", nargs="+", choices=STATES, default=list(STATES))
    parser.add_argument("--size", default="1440x1400")
    parser.add_argument("--scale", type=float, default=1)
    parser.add_argument("--timeout", type=float, default=120)
    args = parser.parse_args()
    try:
        width, height = map(int, args.size.split("x"))
        if not (128 <= width <= 8192 and 128 <= height <= 8192 and 0.5 <= args.scale <= 4
                and width * height * args.scale ** 2 <= 32_000_000):
            raise ValueError()
    except ValueError:
        parser.error("size must be WIDTHxHEIGHT, scale 0.5..4, at most 32 million output pixels")
    original = args.layout.read_bytes()
    base = json.loads(original)
    executable = args.executable.resolve(strict=True)
    image = args.image.resolve(strict=True)
    tokens = args.tokens.resolve(strict=True)
    token_bytes = tokens.read_bytes()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    with executable.open("rb") as binary:
        executable_hash = hashlib.file_digest(binary, "sha256").hexdigest()
    report = {"size": [width, height], "scale": args.scale, "states": {}, "complete": False,
              "executableSha256": executable_hash, "layoutSha256": hashlib.sha256(original).hexdigest(),
              "tokensSha256": hashlib.sha256(token_bytes).hexdigest()}
    initial = None
    try:
        for name in args.states:
            if args.layout.read_bytes() != original or tokens.read_bytes() != token_bytes:
                raise RuntimeError("Layout or theme tokens changed during capture; rerun against one stable design")
            panel, extra = STATES[name]
            design = dict(base)
            design["panelOrder"] = ordered_panels(base["panelOrder"], panel)
            design["preview"] = {"panels": {"open": [panel] if panel else [], "single": False}, **extra}
            layout = output / f"{name}-layout.json"
            layout.write_text(json.dumps(design, indent=2) + "\n", encoding="utf-8")
            environment = dict(os.environ, PHOTOCRAFT_CAMERA_RAW_LAYOUT_FILE=str(layout),
                               PHOTOCRAFT_THEME_FILE=str(tokens),
                               PHOTOCRAFT_CONFIG_DIR=str(output / "profile"), PHOTOCRAFT_LOCALE="en")
            result = subprocess.run([str(executable), "--raw-workspace", "--size", args.size,
                                     "--scale", str(args.scale), "--open", str(image),
                                     "--out", str(output / f"{name}.png"),
                                     "--inspect-out", str(output / f"{name}-state.json")],
                                    env=environment, capture_output=True, text=True, timeout=args.timeout)
            (output / f"{name}.log").write_text(result.stdout + result.stderr, encoding="utf-8")
            if result.returncode:
                raise RuntimeError(f"{name} render failed; see {name}.log")
            with (output / f"{name}.png").open("rb") as png:
                header = png.read(24)
            if header[:8] != b"\x89PNG\r\n\x1a\n":
                raise RuntimeError(f"{name}: native render did not write a PNG")
            dimensions = list(struct.unpack(">II", header[16:24]))
            expected = [math.floor(value * args.scale + 0.5) for value in (width, height)]
            if dimensions != expected:
                raise RuntimeError(f"{name}: rendered {dimensions}, expected {expected}")
            state = json.loads((output / f"{name}-state.json").read_text(encoding="utf-8"))["result"]
            if not includes_view(state["cameraRaw"]["workspace"], design["preview"]):
                raise RuntimeError(f"{name}: rendered view did not match the requested state")
            if state["cameraRawDesign"]["panelOrder"] != design["panelOrder"]:
                raise RuntimeError(f"{name}: panel order did not match the requested layout")
            identity = photo_identity(state)
            if initial is None:
                initial = identity
            if identity != initial:
                raise RuntimeError(f"{name}: photo parameters or document identity differed between views")
            report["states"][name] = {"matchedRequestedView": True, "photo": identity,
                                     "actualPreview": state["cameraRaw"]["workspace"], "imageSize": dimensions}
            print(name, "rendered and verified", flush=True)
        report["complete"] = True
    finally:
        (output / "capture-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
