# Prompt for the local agent: MG-03 (verify the LaMa ONNX model)

Run from `E:\CraftML\work` (not inside photocraft) with `codex --profile strata`. Paste everything below the line.

---

Task MG-03 from `photocraft/agents/TASKS.md`. Work only inside a new folder `E:\CraftML\work\lama-check\`. Do not touch the `photocraft` folder except to update MG-03's status line in `photocraft/agents/TASKS.md` at the end.

Goal: verify one ONNX model file and record the facts, so a later task can build an inference runner from them.

1. Download `https://huggingface.co/Carve/LaMa-ONNX/resolve/a3ee2fca54baebec351b8fa7786154ffa7555aa6/lama_fp32.onnx` to `lama-check\lama_fp32.onnx` (about 208 MB).
2. Compute its SHA-256 (`Get-FileHash -Algorithm SHA256`). Expected: `1faef5301d78db7dda502fe59966957ec4b79dd64e16f03ed96913c7a4eb68d6`. Report a mismatch clearly; don't stop.
3. Inspect the model and record every input and output: name, shape, dtype. Use Python with the `onnx` and `onnxruntime` packages if Python is installed (`pip install --user onnx onnxruntime numpy pillow`). Never load `.ckpt`, `.pt`, `.pth` or `.bin` files, and never use `pickle` or `torch.load`.
4. Make a 512×512 test image (a gradient with a few coloured shapes) and a mask (white rectangle in the middle, black elsewhere). Run the model once on the CPU. Find out from the outputs, by experiment, the expected input value range (0–1 or 0–255), the mask convention (1 = fill?), and the output value range. Save `input.png`, `mask.png`, `output.png`.
5. Time 5 runs on the CPU and report the median.
6. Write `lama-check\lama.md` with: URL, revision, file size, SHA-256 (expected vs actual), inputs/outputs table, value ranges and mask convention, CPU median time, and this licence note: "Tagged Apache-2.0; port of big-lama (advimman/lama, Apache-2.0); trained on Places365, whose images are licensed for non-commercial research. Commercial use is Erik's decision (TASKS D2)."
7. Keep the script you used as `lama-check\check_lama.py`.

At the end set MG-03 in `photocraft/agents/TASKS.md` to `done` or `blocked: <reason>`, and add a three-line note at the top of `photocraft/agents/HANDOFF.md` pointing to `lama-check\lama.md`.
