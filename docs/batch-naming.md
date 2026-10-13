# Batch output naming

Select a recorded action, then open **File > Automate > Batch**. Set the source (a folder, with
**Include All Subfolders** mirroring its tree under the output folder, or the open documents),
the output folder and the format. **File Naming** is a template for the output file names; leave
it blank to keep the source names (`{name}.{ext}`). The tokens follow Photoshop's File Naming
fields:

| Token | Value |
| --- | --- |
| `{name}`, `{NAME}`, `{Name}` | The input's name without its extension: as is, upper case, capitalised |
| `{serial}` | The input's position plus **Starting serial** (default 1), padded to three digits |
| `{serial:1}` … `{serial:4}` | The same serial padded to that many digits (never truncated: `{serial:2}` after 99 is `100`) |
| `{letter}`, `{LETTER}` | The serial as letters: `a` … `z`, `aa`, `ab` … (upper case with `{LETTER}`) |
| `{date}` | The UTC date at the start of the batch, `YYYY-MM-DD` |
| `{date:mmddyy}`, `{date:mmdd}`, `{date:yyyymmdd}`, `{date:yymmdd}`, `{date:yyddmm}`, `{date:ddmmyy}` | The same date in Photoshop's formats |
| `{ext}`, `{EXT}` | The output format's extension, lower or upper case |

Any other text is literal; braces in an input's name are literal too. A template without `{ext}`
names a stem and gets the extension of the output format, so `{name}_{serial}` with starting
serial 7 writes `photo_007.png` and `photo_008.png` for two PNG inputs named `photo.png`. Folder
inputs use the listing order (each folder's files by name, then its subfolders), and an explicit
input array keeps its order. A failed input keeps its position, so later names do not shift.

The whole run is planned before anything is written. Unknown tokens, unmatched braces, a
template that is not a string or is longer than 255 bytes, path separators, `..`, reserved
filename characters, a starting serial outside 1 to 4,294,967,295, and a run whose serials would
pass that are rejected first, with no output folder created. So is a template whose output
would replace another input before that input is read (`{serial}` over the input folder would
turn `002.png` into `001.png`); choose another destination or template. Per-file read, action and
save failures still appear in the batch's `errors` result (and in the error log with **Log Errors
To File**; **Stop For Errors** stops at the first and reports the rest as `skipped`). When two
results share a name, including a case-only difference, the first is kept and the later
collision is reported.

The same options are available on `file.automate.batch` (and `file.scripts.imageProcessor`, the
lens-correction batch and the CLI's `batch`) through the engine command interface:

```json
{
  "input": ["first/photo.png", "second/photo.png"],
  "output": "exports",
  "steps": [["image.adjustments.invert", {}]],
  "format": "png",
  "naming": "{name}_{serial}",
  "startingSerial": 7
}
```

The CLI's `batch --naming <template> --start-serial <n>` takes the same template.
