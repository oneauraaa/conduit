# Relay v1.0

Relay is a QLoRA adapter for Ornith 1.5 9B tuned on Conduit MCP tool use and Auto Mode approval behavior.

The existing `Ornith-1.5-9B-Q4_K_M.gguf` is kept as a reference. Training uses the Hugging Face BF16 checkpoint because standard QLoRA tooling cannot train a GGUF file directly. The pipeline loads that checkpoint in 4-bit NF4, trains only language-model LoRA layers, merges the adapter, and exports a separate Q4_K_M GGUF.

## Requirements

- NVIDIA GPU with CUDA; the tested target is an RTX 5070 Ti with 16 GB VRAM.
- Unsloth Studio or its Python environment. This machine currently has Unsloth 2026.8.18, PyTorch 2.11.0+cu130, Transformers 5.5.0, TRL 0.23.1, and bitsandbytes 0.50.1.
- At least 25 GB free disk for the Hugging Face safetensors cache, adapter, merged checkpoint, and temporary conversion files. More is useful during export.
- Network access to download `ornith-ai/Ornith-1.5-9B` on the first run.

The script locates the Studio interpreter automatically when launched with `python` from that environment. To use it explicitly:

```bash
PY="$HOME/.unsloth/studio/unsloth_studio/bin/python"
```

## Commands

Run these from the repository root:

```bash
$PY relay-v1.0/train_relay.py prepare
$PY relay-v1.0/train_relay.py smoke
$PY relay-v1.0/train_relay.py train
$PY relay-v1.0/train_relay.py export
$PY relay-v1.0/train_relay.py evaluate
```

`prepare` is CPU-only and validates the schema and JSONL. `smoke` loads the model and performs a very short bounded training path. Run it before a long training job. `train` writes the LoRA adapter to `relay-v1.0/output/adapter`. `export` creates the merged checkpoint and `relay-v1.0/output/Ornith-1.5-9B-Relay-v1.0-Q4_K_M.gguf`. `evaluate` writes `relay-v1.0/output/evaluation.json`.

The initial configuration uses 2,048 tokens, batch size 1, gradient accumulation 8, NF4 4-bit loading, gradient checkpointing, and a paged 8-bit optimizer. If the smoke run reports an out-of-memory error, rerun with `--max-seq-length 1024` before changing the model or quantization target.

Auto Mode examples teach Relay to request approval before catalogued risky tools such as `run_shell`, `quit_app`, and `clipboard_write`. Conduit remains the runtime authority: a model response never bypasses the app's approval gate.
