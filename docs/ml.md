# Machine learning (SAM 2 Object Selection)

Object Selection (click, hover preview and rectangle) uses **SAM 2** (Segment Anything 2, Meta,
Apache-2.0) when its model files and ONNX Runtime are installed, and falls back to the classical
GrabCut finder otherwise. Nothing is bundled: PhotoCraft works without any of this.

## How it works

`crates/ml` (`photocraft-ml`, L4) wraps the two ONNX halves through the `ort` crate with
`load-dynamic`, so builds download nothing and wasm builds get a stub.

1. **Encode** the image once (resized to 1024², ImageNet-normalised): about 0.1 s (Small) to 0.2 s
   (Base+) on an RTX 2080. `engine::sam_object` caches the embedding for the document's pixels.
2. **Decode** each click or rectangle against the cached embedding: about 6-13 ms. The model
   returns three candidate masks; the one with the best predicted IoU wins. A click whose mask
   covers more than 60% of the canvas is the backdrop and selects nothing (as before).
3. The 256² mask logits are sampled bilinearly per pixel, then **Refine Edge** (`matting`) snaps
   the edge to the image at full resolution.

The first click also loads the model (about 0.7 s); the Object Selection tool warms it up in the
background. Use extra clicks (⇧ adds, ⌥ subtracts) or a rectangle for multi-part objects: the model
picks a part or the whole according to its own score.

## Installing

Assets root: `PHOTOCRAFT_ML_DIR`, else `<local data>/PhotoCraft/ml`.

```text
models/sam2/sam2_hiera_{tiny,small,base_plus,large}.{encoder,decoder}.onnx
runtime/ort-gpu/onnxruntime/capi/onnxruntime.dll     (or set ORT_DYLIB_PATH)
runtime/cuda13/nvidia/cu13/bin/x86_64, runtime/cuda13/nvidia/cudnn/bin   (optional, NVIDIA GPU)
```

* Models: `vietanhdev/segment-anything-2-onnx-models` on Hugging Face (public, Apache-2.0).
* ONNX Runtime 1.31 (`ort` 2.0.0-rc.14 matches it): the `onnxruntime-gpu` wheel's `onnxruntime/capi`
  folder works; the GPU build needs CUDA 13 and cuDNN 9 (the `nvidia-*-cu13` wheels; the loader
  preloads every library in those folders, no `PATH` change). Without them it runs on the CPU
  (about 0.75 s per encode for Small).
* `PHOTOCRAFT_SAM2=0` disables it; `PHOTOCRAFT_SAM2_SIZE=tiny|small|base_plus|large` (default `small`).

## Remote AI service (optional)

PhotoCraft can use a remote AI service for two things: Object Selection (bigger SAM models) and
**Decompose into Layers** (splitting an image into layers with Qwen-Image-Layered). It is **off
unless set up** and never required; nothing is uploaded until a URL and key are set.

### Setting it up

Edit › Preferences › **AI Integrations**:

* **Service URL**: the API base, e.g. `https://host/photocraft/v1`. Must be `https` (plain `http` is
  accepted only for localhost, so a key never travels in clear text).
* **API key** (Save Key / Remove Key): kept in the operating system's credential store (Windows
  Credential Manager, macOS keychain, Linux kernel keyring), **never** in the preferences file, which
  is plain JSON and readable over the agent interface. The engine holds it in memory only. If the
  store refuses it, the key is used for that session only and the dialog says so.
* **Test Connection**: asks the service what it offers (`GET /models`) with the URL and key as typed,
  on a worker thread; it shows the models and queue length, or a clear error.
* Switches: **Use for Object Selection**, **Use for Decompose into Layers**, and the Decompose
  defaults (layers 2-10, detail 640 or 1024, quality steps 10-60, layer type).

Headless use (CLI, scripts, MCP): without a URL in the preferences the engine reads
`PHOTOCRAFT_REMOTE_URL` and `PHOTOCRAFT_REMOTE_KEY`. Preferences win when a URL is set there.

### Object Selection

Order: remote, then the local model, then the built-in finder. The contract is `POST <base>/segment`,
`multipart/form-data` with `image` (JPEG) and `prompts` (JSON: `{"model":"sam2_large",
"points":[{"x","y","positive"}],"box":[x0,y0,x1,y1],"multimask":false}`), answering
`{"masks":[{"png_base64","score","bbox"}],"width","height"}` with 8-bit greyscale PNG masks at the
uploaded size.

What is uploaded: the *working image* (the active layer, or the composite with Sample All Layers,
about 1 MP as JPEG), once per document state; the same bytes are re-sent so the server's embedding
cache hits. A failed request (network, 401, 5xx) is logged and skips the remote for 60 s so hovering
never stalls; the local model answers meanwhile. The mask is finished like a local one (backdrop
rejection, bilinear upsampling, Refine Edge). The Object Selection options bar shows **who answered
last** ("Remote service", "Local model" or "Built-in finder"), and the log records each change.

Measured (reference server, SAM 2.1 Large): about 250 ms for the first selection on an image including
upload, about 70 ms after; masks match local SAM 2 Small to IoU 0.99+.

### Decompose into Layers

Properties › Quick Actions on a pixel layer, or the command `layer.decompose`
(`{"layers","resolution","steps","mode","prompt","seed","sampleAllLayers","hideOriginal"}`). It is a
**background job** (a 4-layer, 640 px, 40-step run takes about 5 minutes on the reference server; the
job panel shows "Waiting in the queue (n ahead)", "Generating layers: step 12/40, about 3 min left",
and Esc cancels it on the server too). The layers arrive in a group above the source layer in **one
undo step**; the source is hidden, not deleted.

* **Generative**: the model's own RGBA layers, including content completed behind occluders (road
  behind a car). The colours are synthesised, so the stack only approximates the photo (the manifest's
  `composite_error` says how closely; 0.03 on the test photo).
* **Mask**: the model's alpha as layer masks over copies of the original pixel layer, edges snapped to
  the photo with Refine Edge: no pixel is invented and the stack reproduces the photo exactly.

Contract: `POST /decompose` (multipart: `image`, `layers`, `resolution`, `steps`, `cfg`, `mode`,
optional `seed`, `prompt`) returns a job id; `GET /jobs/{id}` (state, queue position, step, ETA);
`GET /jobs/{id}/result` (zip: `manifest.json` + `layer_NN.png`); `DELETE /jobs/{id}`. Result archives
are read with entry-count and size limits, and names are lookup keys only, never paths.

Measured on the reference server (Qwen-Image-Layered bf16, GB10): 4 layers at 640 px about 7.4 s per
step (5 min at 40 steps), at 1024 px about 25 s per step (17 min). The defaults are 640 px, 40 steps;
the live test uses 10 steps (82 s) for a quick look.

### Object to Layer (fill what the object leaves behind)

Right-click a selection (Object Selection, Marquee, Lasso… any selection tool) › **Object to Layer**,
or the command `layer.objectToLayer`
(`{"layer","method","fill","grow","prompt","seed"}`). The selected pixels move onto a new layer above
the source, and the hole they leave is **filled with plausible background by the AI service's
inpainting model**, so the photo underneath looks as if the object had never been there. One undo step.

* **Choosing the model:** Edit › Preferences › AI Integrations › **Fill method**. The list comes from
  the service (`GET /models` → `inpaint_models`, refreshed when the page opens and by Test Connection),
  so a model the server adds appears without an app update. The reference service offers `fast` (LaMa,
  invents nothing, about 1.3 s), `sdxl` and `juggernaut` (SDXL inpainting, about 8 s; they invent
  content and follow `prompt`). The `method` parameter overrides the preference for one run.
* **What is sent:** the source layer at about 1.6 MP (JPEG) and the selection, grown by `grow` (default
  8 uploaded pixels) so halos and shadows at the rim are covered, as a greyscale mask. The service
  returns the whole image with the area filled; PhotoCraft blends it back **only inside the grown
  mask** (bilinear, at the layer's full resolution), so no other pixel changes. The fill is therefore
  as sharp as the upload: on a very large photo a big filled patch can look slightly soft.
* **Without a service**, with **Use for Object to Layer fill** off, or when the service fails (network,
  401, queue, timeout) the object is still cut out and the hole is left as Layer via Cut leaves it
  (transparent, or the background colour on the Background layer). The command's answer carries a
  `warning` saying why; Esc cancels the job on the server too.

Contract (service side): `POST /inpaint` (multipart `image`, `mask` PNG of the same size with 255 =
fill, `mode` = `fast`|`generative`, `model`, `dilate`, `steps`, `prompt`, `seed`) returns a job id;
the job is polled like Decompose; `GET /jobs/{id}/result` is a zip with `manifest.json` and
`fill.png` at the uploaded size, identical to the input outside the mask. The spec for the server
side is `photocraft-gpu-server-SPEC-addendum-inpaint.md`.

Live check against a real service: `PHOTOCRAFT_REMOTE_TEST=1 PHOTOCRAFT_REMOTE_URL=…
PHOTOCRAFT_REMOTE_KEY=… PHOTOCRAFT_TEST_IMAGE=photo.jpg PHOTOCRAFT_TEST_POINT=x,y
PHOTOCRAFT_TEST_OUT=dir cargo test -p photocraft-engine --release --lib object_to_layer_live --
--ignored --nocapture` dumps the filled background for each method.

### Testing the remote features

* Unit tests use a loopback mock server and never touch the network.
* Live: `PHOTOCRAFT_REMOTE_TEST=1 PHOTOCRAFT_REMOTE_URL=... PHOTOCRAFT_REMOTE_KEY=...
  PHOTOCRAFT_TEST_IMAGE=photo.jpg PHOTOCRAFT_TEST_OUT=dir cargo test -p photocraft-engine --release
  --lib decompose_live -- --ignored --nocapture` (and `sam2_live_photo_dump` for selection).

## Measured (RTX 2080, 8 GB, ORT 1.31 CUDA)

| Model | Encode | Decode |
|---|---|---|
| SAM 2 Small | 106 ms | 13 ms |
| SAM 2 Base+ | 186 ms | 11 ms |
| SAM 3 tracker (fp16, 1008²) | 317 ms | 11 ms |

First click on a 12 MP photo, including model load, encode, decode and refine: about 1.0-1.2 s;
later objects on the same pixels are about 10 ms. Do not load several models on an 8 GB GPU at once
(encode times collapse to seconds).

## Quality

On public test photos and a set of real 12 MP phone photos the classical finder found nothing on
most clicks (dog, can, bag, legs, ladder) or only part of the object (truck without wheels and
windows); SAM 2 found them all with clean edges. SAM 3's tracker ONNX
(`onnx-community/sam3-tracker-ONNX`) gave the same masks for point and box prompts at 3x the encode
time and 6x the weights, so SAM 2 stays the default. SAM 3's text-prompt "concepts" are not in
that export, and `facebook/sam3` is gated under Meta's own SAM License (not Apache-2.0).

## Tests

* `cargo test -p photocraft-ml` (pure maths) and the engine tests stay on the classical path unless
  `PHOTOCRAFT_SAM2=1` (the unit tests are deterministic).
* Real model, opt-in: `PHOTOCRAFT_ML_DIR=<assets> cargo test -p photocraft-ml --release --test
  sam2_live -- --ignored --nocapture`, and `PHOTOCRAFT_SAM2=1 cargo test -p photocraft-engine --release
  --lib sam2_live -- --ignored --nocapture`.

## Next

Preferences panel to download and manage local models; use SAM 2 for Select Subject (box of the canvas
plus saliency points) and for the Quick Selection brush; background removal; a model-size choice in
the UI; CPU thread and memory limits; SAM 3 text prompts if an openly licensed export appears.
