# Publication figures

Tools for preparing figures for journals (for example CT and MRI panels at 300 dpi): building a
multi-panel figure, labelling panels, marking findings with arrows and markers, and exporting
print-ready files. None of these has a Photoshop counterpart; each is an engine command, so the
same work can be scripted through the control channel or MCP.

## Make Figure (File › Automate › Make Figure…, `figure.make`)

Lays several images out as one new document at a journal width and resolution:

- **Width:** single column (85 mm), 1.5 column (120 mm), double column (175 mm) or a custom width
  in mm, at 300 dpi by default.
- **Layout:** *Rows* scales every image in a row to the same height and fills the width exactly,
  so images of different sizes line up without cropping. *Grid* fits each image into equal cells.
- **Gutters:** the lines between panels, with a width in mm and a colour (white or black is usual).
- **Labels:** a, b, c / A, B, C / (a) / (A) / a) / A), in any corner, sized in points, bold, with an
  optional contrasting outline. Labels are editable type layers; each image is its own pixel
  layer, so a panel can still be nudged, cropped or annotated.

The dialog takes a folder and places its images in name order; `figure.make` also takes an
explicit list of files.

## Arrow tool (U)

See [Shape stroke options](shape-strokes.md) for the arrowhead model. For print work the width is
shown in **points** at the document's resolution (2 pt is 8 px at 300 dpi; documents under 144 dpi
use 2 px per point so screenshot arrows stay visible). Quick styles include solid, double-headed,
curved, dashed and dotted arrows and two outlined styles. **Outline** adds a thin contrasting
Stroke layer style (black on light colours, white on dark ones) so an arrow reads on dark and
bright tissue alike; **Curved** makes new arrows start with a gentle bend (drag the blue middle
handle to change it).

## Marker tool (U)

Click to stamp a star, asterisk, circle or triangle centred on the click, sized in points, in any
colour, with the same optional outline. Each marker is a shape layer.

## Export for Publication (File › Export › Export for Publication…, `file.export.publication`)

Writes the flattened image as TIFF (LZW, lossless) and/or PNG in one step, with the resolution
stored in the files. It can also resample to a journal column width at 300 or 600 dpi, and warns
when that enlarges the image more than 1.5×. The document itself is not changed. Files go next to
the document by default, named `<document>_<dpi>dpi.tif` / `.png`.
