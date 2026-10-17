"""Audit the supported reference views without treating partial coverage as parity.

Every registration is fixed for the entire compared panel. Reference pixels and
generated overlays stay outside Git. The report always lists all 19 references,
including screens and shared elements that still have no individual audit.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path


# Current goal numbering; source hashes and crops live in the region specs.
# Multiple sections from a single image share the same dock scale/translation.
REGISTRATIONS = [
    (1, "info", "info", [2260, 64, 2800, 948]),
    (4, "edit", "edit", [2260, 64, 2800, 2470]),
    (5, "curve-mixer", "curve-mixer", [2260, 520, 2800, 2578]),
    (6, "grading", "grading-detail", [2260, 520, 2800, 1506]),
    (6, "detail", "grading-detail", [2260, 1506, 2800, 1996]),
    (7, "lower-panels", "lens-calibration", [2260, 520, 2800, 2612]),
    (8, "collapsed", "collapsed", [2260, 64, 2800, 1324]),
    (12, "red-eye", "red-eye", [2260, 360, 2800, 740]),
    (13, "snapshots", "snapshots", [2260, 64, 2800, 548]),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--references", type=Path, required=True)
    parser.add_argument("--capture", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    inventory = json.loads((repo / "design/camera-raw/reference-inventory.json").read_text(encoding="utf-8"))
    captured = json.loads((args.capture / "capture-report.json").read_text(encoding="utf-8"))
    if not captured.get("complete") or captured.get("scale") != 2.0 or captured.get("size") != [1440, 1400]:
        parser.error("these fixed registrations require a complete, verified 1440x1400 capture at scale 2")
    report = {"referenceSet": inventory["referenceSet"], "capture": captured,
              "fullEveryElementParityVerified": False, "references": [],
              "notes": "Only named rectangles are audited. Unlisted controls, shared chrome, full windows and unavailable behavior remain incomplete."}
    for reference in inventory["references"]:
        path = args.references / reference["file"]
        with path.open("rb") as source_file:
            digest = hashlib.file_digest(source_file, "sha256").hexdigest()
        if digest != reference["sha256"]:
            parser.error(f"reference {reference['image']} does not match the inventoried image hash")
        files = {f"{section}-element-regions.json" for number, section, _, _ in REGISTRATIONS if number == reference["image"]}
        if reference.get("elementRegionsFile"):
            files.add(reference["elementRegionsFile"])
        specifications = []
        for name in sorted(files):
            specification = json.loads((repo / "design/camera-raw" / name).read_text(encoding="utf-8"))
            if specification["sourceSha256"] != digest:
                parser.error(f"{name} specification belongs to a different reference")
            specifications.append({"file": name, "elementCount": len(specification["elements"]), "audited": False})
        report["references"].append({"image": reference["image"], "sha256": digest,
                                     "scope": reference["scope"], "audits": [],
                                     "regionSpecifications": specifications,
                                     "sharedElementsUnverified": reference.get("sharedElements", []),
                                     "fullReferenceVerified": False})
    args.output.mkdir(parents=True, exist_ok=False)
    for number, section, state, candidate_rect in REGISTRATIONS:
        if not captured["states"].get(state, {}).get("matchedRequestedView"):
            parser.error(f"native {state} view was not verified in this capture")
        specification_path = repo / f"design/camera-raw/{section}-element-regions.json"
        specification = json.loads(specification_path.read_text(encoding="utf-8"))
        reference = report["references"][number - 1]
        if specification["sourceSha256"] != reference["sha256"]:
            parser.error(f"{section} specification belongs to a different reference")
        output = args.output / f"ref{number:02}-{section}"
        subprocess.run([sys.executable, str(repo / "scripts/compare_camera_raw.py"),
                        "--source", str(args.references / f"image-{number}.png"),
                        "--candidate", str(args.capture / f"{state}.png"),
                        "--source-rect", *map(str, specification["sourceRect"]),
                        "--candidate-rect", *map(str, candidate_rect),
                        "--width", str(specification["normalizedWidth"]), "--regions", str(specification_path),
                        "--output", str(output)], check=True)
        audit = json.loads((output / "element-audit.json").read_text(encoding="utf-8"))
        reference["audits"].append({"section": section, "viewer": f"{output.name}/index.html", **audit})
        for item in reference["regionSpecifications"]:
            if item["file"] == specification_path.name:
                item["audited"] = True
    report["auditedElementCount"] = sum(len(audit["elements"]) for ref in report["references"] for audit in ref["audits"])
    report["referencesWithoutIndividualAudits"] = [ref["image"] for ref in report["references"] if not ref["audits"]]
    report["specifiedElementCount"] = sum(item["elementCount"] for ref in report["references"] for item in ref["regionSpecifications"])
    report["specifiedElementsWithoutAudits"] = sum(item["elementCount"] for ref in report["references"] for item in ref["regionSpecifications"] if not item["audited"])
    (args.output / "reference-audit.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: report[key] for key in ["auditedElementCount", "referencesWithoutIndividualAudits", "fullEveryElementParityVerified"]}), flush=True)


if __name__ == "__main__":
    main()
