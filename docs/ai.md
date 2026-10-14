# AI features: Select Subject model and Generative Fill

Two features use machine learning. Both are optional, both keep working without a network
(Select Subject falls back to the classical algorithm), and nothing is uploaded unless the user
presses Generate.

## Select Subject (local model)

- Model: IS-Net "general use" (DIS, Xuebin Qin et al., ECCV 2022, Apache-2.0), exported to ONNX by
  the rembg project (MIT). 178 MB, SHA-256 pinned in `crates/ml/src/models.rs`. It is **not** in the
  repository.
- Runtime: [`tract`](https://github.com/sonos/tract) (pure Rust, Apache-2.0 / MIT) on the CPU,
  multithreaded. About 4.5 s for a 1024 x 1024 input on an 8-core machine.
- First use downloads the model into the user's cache folder (`%LOCALAPPDATA%\photocraft\models`,
  `~/Library/Caches/photocraft/models`, `$XDG_CACHE_HOME/photocraft/models`). `PHOTOCRAFT_MODEL_DIR`
  overrides the folder: copy `isnet-general-use.onnx` there for an offline install.
- `select.subject` and `layer.removeBackground` take `engine`: `auto` (default; the model when it is
  on disk, or when the app enabled auto-download, otherwise the classical algorithm), `model`
  (always the model, downloading it if needed, errors are reported) or `classic`.
  The desktop app enables auto-download; tests, the CLI and scripts do not.
- `PHOTOCRAFT_ML=off` disables the model altogether.
- The matte is stretched over the canvas and becomes the selection (`algo::segment::matte`).

## Generative Fill

`edit.generativeFill` (Edit › Generative Fill…, and a button in the Quick Selection, Object
Selection and Magic Wand option bars): a crop of the composite around the selection and a mask go to
the chosen provider; the answer becomes a new pixel layer whose alpha is the selection (one undo
step, the original is untouched). Empty prompt = remove the selected content.

| Provider id | Service | Notes |
|---|---|---|
| `cloudflare` | Workers AI, `@cf/runwayml/stable-diffusion-v1-5-inpainting` | Free daily allowance; needs account id + API token. 512 px working size. |
| `sdwebui` | AUTOMATIC1111 / Forge (`/sdapi/v1/img2img`) | Local, free. Start it with `--api`. |
| `gemini` | `gemini-2.5-flash-image` through `generateContent` | The mask is sent as a second image and described in the prompt (Gemini has no mask parameter). |
| `openai` | `/v1/images/edits`, `gpt-image-1` | Base URL and model are editable, so compatible servers work. |
| `stability` | `/v2beta/stable-image/edit/inpaint` | Paid credits. |

Credentials are saved with `ai.settings` in a **separate file** (`ai-credentials.json` in the user
config folder, `%APPDATA%\photocraft` on Windows; `PHOTOCRAFT_CONFIG_DIR` overrides it), never in the
preferences, so no preferences view or export can carry them. Only the provider choice, server URL
and model are preferences. Without the file (tests, CLI, scripts) saved credentials stay in memory
for the session. They can also come from `PHOTOCRAFT_<PROVIDER>_API_KEY` /
`PHOTOCRAFT_<PROVIDER>_ACCOUNT_ID`. No command returns a key.
Adding a provider is one `Provider` variant and one match arm in `crates/ml/src/genfill.rs`; the
request/response shapes are covered by tests against a local mock server.

The provider request formats are implemented from each service's public documentation and tested
against mock servers; they have not all been run against the live services.
