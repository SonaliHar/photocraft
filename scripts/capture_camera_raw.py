"""Capture matching Camera Raw view states without changing photo parameters.

Uses the existing native preview's authenticated control channel and live layout.
References, credentials and screenshots stay outside version control.
"""
import argparse
import hashlib
import json
import math
import socket
import time
from pathlib import Path


STATES = {
    "edit": (None, {"panels": {"open": ["Light", "Color", "Effects", "Curve"], "single": False}}),
    "curve": ("Curve", {"curveChannel": "parametric"}),
    "curve-mixer": ("Curve", {"panels": {"open": ["Curve", "Color Mixer", "Optics"], "single": False}, "curveChannel": "parametric", "mixer": "color", "band": 5}),
    "mixer": ("Color Mixer", {"mixer": "color", "band": 5}),
    "grading": ("Color Grading", {"grading": "threeWay"}),
    "grading-detail": ("Color Grading", {"panels": {"open": ["Color Grading", "Detail", "Optics"], "single": False}, "grading": "threeWay"}),
    "detail": ("Detail", {}),
    "optics": ("Optics", {}),
    "lens-calibration": ("Optics", {"panels": {"open": ["Optics", "Lens Blur", "Calibration"], "single": False}}),
    "collapsed": (None, {}),
    "info": (None, {"info": True}),
    "grid-info": (None, {"info": True, "grid": True}),
    "snapshots": (None, {"tab": "snapshots"}),
}


def ordered_panels(base_order, first_panel):
    """Start at a later section without inserting earlier headers between its neighbors."""
    if first_panel is None:
        return list(base_order)
    index = base_order.index(first_panel)
    return [*base_order[index:], *base_order[:index]]


def includes_view(actual, requested):
    """Defaulted native fields may be extra; every requested field must match."""
    if isinstance(requested, dict):
        return isinstance(actual, dict) and all(key in actual and includes_view(actual[key], value) for key, value in requested.items())
    if isinstance(requested, float) and isinstance(actual, (int, float)) and not isinstance(actual, bool):
        return math.isclose(actual, requested, rel_tol=1e-6, abs_tol=1e-6)
    return actual == requested


def request(port, token, method, params, timeout):
    with socket.create_connection(("127.0.0.1", port), timeout=timeout) as connection:
        stream = connection.makefile("rwb")
        for name, arguments in [("auth", {"token": token}), (method, params)]:
            stream.write((json.dumps({"id": name, "method": name, "params": arguments}) + "\n").encode())
            stream.flush()
            line = stream.readline()
            if not line:
                raise RuntimeError(f"Control connection closed during {name}")
            reply = json.loads(line)
            if not reply.get("ok"):
                raise RuntimeError(f"{name}: {reply.get('error', 'request failed')}")
        return reply["result"]


def photo_identity(state):
    raw = state.get("cameraRaw")
    if raw is None:
        raise RuntimeError("Open Camera Raw in the isolated native preview first")
    document = state.get("document") or {}
    return {"params": raw["params"], "revision": document.get("revision"),
            "document": document.get("path"), "name": document.get("name"),
            "layer": raw.get("layer"), "sourceSize": raw.get("sourceSize")}


def capture(args):
    layout = args.layout.resolve()
    original = layout.read_bytes()
    base = json.loads(original)
    token = args.token_file.read_text(encoding="utf-8").strip()
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "layout-before.json").write_bytes(original)
    last_written = original
    call = lambda method, params: request(args.port, token, method, params, args.timeout)
    initial = photo_identity(call("ui.inspect", {}))
    report = {"layoutSha256": hashlib.sha256(original).hexdigest(),
              "photoBefore": initial, "states": {}, "restored": False}
    try:
        for name in args.states:
            if layout.read_bytes() != last_written:
                raise RuntimeError("Layout changed externally; preserving that edit")
            panel, extra = STATES[name]
            design = dict(base)
            design["panelOrder"] = ordered_panels(base["panelOrder"], panel)
            design["preview"] = {"panels": {"open": [panel] if panel else [], "single": False}, **extra}
            last_written = (json.dumps(design, indent=2) + "\n").encode()
            layout.write_bytes(last_written)
            deadline = time.monotonic() + args.timeout
            while True:
                state = call("ui.inspect", {})
                identity = photo_identity(state)
                if identity != initial:
                    raise RuntimeError("Photo parameters or document revision changed during view capture")
                active = state["cameraRawDesign"]
                if (active["panelOrder"] == design["panelOrder"]
                        and includes_view(active["preview"], design["preview"])
                        and active["preview"] == state["cameraRaw"]["workspace"]):
                    break
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"Live layout did not confirm {name}")
                time.sleep(0.15)
            (args.output / f"{name}-state.json").write_text(json.dumps(state, indent=2), encoding="utf-8")
            screenshot = call("ui.screenshot", {"path": str((args.output / f"{name}.png").resolve())})
            after = photo_identity(call("ui.inspect", {}))
            if after != initial:
                raise RuntimeError("Photo changed while capturing screenshot")
            report["states"][name] = {"screenshot": screenshot, "photoUnchanged": True,
                                     "panelOrder": design["panelOrder"], "requestedPreview": design["preview"],
                                     "actualPreview": state["cameraRaw"]["workspace"], "matchedRequestedView": True}
            print(name, screenshot["width"], screenshot["height"], flush=True)
    finally:
        if layout.read_bytes() == last_written:
            layout.write_bytes(original)
            report["restored"] = True
        (args.output / "capture-report.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    if not report["restored"]:
        raise RuntimeError("Layout changed externally; original preserved in layout-before.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--layout", type=Path, default=Path("design/camera-raw/layout.json"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--states", nargs="+", choices=STATES, default=list(STATES))
    parser.add_argument("--timeout", type=float, default=30)
    capture(parser.parse_args())


if __name__ == "__main__":
    main()
