"""Compare observed Camera Raw controls using one uniform scale and explicit crop anchors.

Pillow is required. Keep reference screenshots and generated comparisons outside Git.
Only translate/crop and uniformly scale: never warp controls to make a match look better.
"""
import argparse
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageDraw, ImageStat


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--source-rect", type=int, nargs=4, required=True, metavar=("LEFT", "TOP", "RIGHT", "BOTTOM"))
    parser.add_argument("--candidate-rect", type=int, nargs=4, required=True, metavar=("LEFT", "TOP", "RIGHT", "BOTTOM"))
    parser.add_argument("--width", type=int, default=270)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not 32 <= args.width <= 4096:
        parser.error("width must be between 32 and 4096")
    regions = []
    scales = []
    for path, rect in [(args.source, args.source_rect), (args.candidate, args.candidate_rect)]:
        with Image.open(path) as original:
            left, top, right, bottom = rect
            if not (0 <= left < right <= original.width and 0 <= top < bottom <= original.height):
                parser.error(f"crop is outside {path.name}")
            scale = args.width / (right - left)
            image = original.convert("RGB").crop(rect)
            image = image.resize((args.width, max(1, round(image.height * scale))), Image.Resampling.LANCZOS)
        regions.append(image)
        scales.append(scale)
    height = max(image.height for image in regions)
    aligned = []
    for image in regions:
        canvas = Image.new("RGB", (args.width, height), "#b000b0")
        canvas.paste(image)
        aligned.append(canvas)
    source, candidate = aligned
    overlay = Image.blend(source, candidate, 0.5)
    difference = ImageChops.difference(source, candidate)
    args.output.mkdir(parents=True, exist_ok=True)
    for name, image in [("source", source), ("candidate", candidate), ("overlay-50", overlay), ("difference", difference)]:
        image.save(args.output / (name + ".png"))
    sheet = Image.new("RGB", (args.width * 4, height + 30), "#202020")
    draw = ImageDraw.Draw(sheet)
    for i, (name, image) in enumerate([( "Reference", source), ("Preview", candidate), ("50% overlay", overlay), ("Absolute difference", difference)]):
        sheet.paste(image, (i * args.width, 30))
        draw.text((i * args.width + 6, 8), name, fill="white")
    sheet.save(args.output / "comparison.png")
    report = {"sourceRect": args.source_rect, "candidateRect": args.candidate_rect,
              "uniformScales": scales, "normalizedWidth": args.width,
              "sourceHeight": regions[0].height, "candidateHeight": regions[1].height,
              "meanAbsoluteRgbDifference": ImageStat.Stat(difference).mean,
              "notes": "Top-left crop anchors aligned. Uniform scaling only. Purple marks absent coverage. Difference is diagnostic, not proof of parity; values, fonts and enabled state may differ."}
    (args.output / "alignment.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(args.output / "comparison.png")


if __name__ == "__main__":
    main()
