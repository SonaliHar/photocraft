"""Compare observed Camera Raw controls using one uniform scale and explicit crop anchors.

Pillow is required. Keep reference screenshots and generated comparisons outside Git.
Only translate/crop and uniformly scale: never warp controls to make a match look better.
"""
import argparse
import json
from pathlib import Path
from PIL import Image, ImageChops, ImageDraw, ImageStat


def write_viewer(output, width, height):
    """Keep the reference underneath; opacity never changes image geometry."""
    html = '''<!doctype html>
<html lang="en"><meta charset="utf-8"><title>Camera Raw overlay</title>
<style>
body{margin:0;background:#202124;color:#eee;font:15px system-ui}header{position:sticky;top:0;background:#292a2d;padding:16px;z-index:2;display:flex;gap:18px;align-items:center;flex-wrap:wrap}button,a{color:inherit;background:#414247;border:1px solid #777;border-radius:4px;padding:7px;cursor:pointer}input{vertical-align:middle}main{padding:24px;overflow:auto}#frame{position:relative;width:WIDTHpx;height:HEIGHTpx;transform-origin:top left;background:#b000b0}#frame img{position:absolute;inset:0;width:100%;height:100%}#ours{opacity:.5}p{padding:0 24px;color:#bbb;max-width:900px}output{font-variant-numeric:tabular-nums}
</style>
<header><strong>ACR underneath · PhotoCraft above</strong>
<label>PhotoCraft opacity <input id="opacity" type="range" min="0" max="100" value="50"><output id="amount">50%</output></label>
<button id="source">ACR only</button><button id="candidate">PhotoCraft only</button><button id="overlay">50% overlay</button><button id="blink" aria-pressed="false">Blink</button>
<label>Zoom <select id="zoom"><option value=".5">50%</option><option value="1" selected>100%</option><option value="2">200%</option><option value="3">300%</option></select></label>
<a href="difference.png" target="_blank" rel="noopener">Difference</a><a href="comparison.png" target="_blank" rel="noopener">Side by side</a><a href="alignment.json" target="_blank" rel="noopener">Alignment data</a></header>
<p>Matching crop anchors, uniform scaling only. Purple marks absent image coverage. Different values, image content, fonts and unavailable controls remain visible. This overlay measures appearance, not processing parity.</p>
<main><div id="space"><div id="frame"><img src="source.png" alt="ACR reference underneath"><img id="ours" src="candidate.png" alt="PhotoCraft above"></div></div></main>
<script>
const ours=document.getElementById('ours'),slider=document.getElementById('opacity'),amount=document.getElementById('amount');let timer=null;
function setOpacity(v){slider.value=v;ours.style.opacity=v/100;amount.value=v+'%'}
function stop(){clearInterval(timer);timer=null;document.getElementById('blink').setAttribute('aria-pressed','false')}
slider.oninput=()=>{stop();setOpacity(slider.value)};
document.getElementById('source').onclick=()=>{stop();setOpacity(0)};document.getElementById('candidate').onclick=()=>{stop();setOpacity(100)};document.getElementById('overlay').onclick=()=>{stop();setOpacity(50)};
document.getElementById('blink').onclick=()=>{if(timer){stop();setOpacity(50)}else{document.getElementById('blink').setAttribute('aria-pressed','true');timer=setInterval(()=>setOpacity(slider.value==100?0:100),650)}};
document.getElementById('zoom').onchange=e=>{let z=Number(e.target.value);document.getElementById('frame').style.transform='scale('+z+')';document.getElementById('space').style.width=(WIDTH*z)+'px';document.getElementById('space').style.height=(HEIGHT*z)+'px'};
</script></html>'''
    (output / "index.html").write_text(html.replace("WIDTH", str(width)).replace("HEIGHT", str(height)), encoding="utf-8")


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
    write_viewer(args.output, args.width, height)
    print(args.output / "comparison.png")


if __name__ == "__main__":
    main()
