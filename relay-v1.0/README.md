# Relay v1.0

Relay is a QLoRA adapter for Ornith 1.5 9B tuned on Conduit's 45-tool catalog,
including the optional built-in browser and its permission precedence rules.

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
$PY relay-v1.0/train_relay.py train --resume-from-checkpoint relay-v1.0/output/adapter/checkpoint-106
$PY relay-v1.0/train_relay.py evaluate-model
$PY relay-v1.0/train_relay.py export
$PY relay-v1.0/train_relay.py evaluate
```

`prepare` is CPU-only and validates the schema and JSONL. It writes a deterministic corpus built from curated scenarios, keeping host and policy families together while preserving tool and approval coverage in both splits. `smoke` loads the model and performs a very short bounded training path. Run it before a long training job. `train` writes the LoRA adapter to `relay-v1.0/output/adapter`; an interrupted full run can resume from an explicit Trainer checkpoint without losing optimizer, scheduler, or RNG state. `export` creates the merged checkpoint and `relay-v1.0/output/Ornith-1.5-9B-Relay-v1.0-Q4_K_M.gguf`. `evaluate` writes a structural dataset audit to `relay-v1.0/output/dataset_audit.json`; it does not claim model performance until an adapter or endpoint is actually evaluated.

The corpus is assembled by `relay-v1.0/relay_examples.py` and validated by `relay-v1.0/relay_dataset.py`. Training examples use sequential one-call/one-result turns, exact tool schemas, observed window IDs, approval metadata, and native Ornith tool-call syntax. The trainer uses completion-only targets so user messages and synthetic tool results provide context without becoming text the model is trained to generate.

The initial configuration uses 2,048 tokens, batch size 1, gradient accumulation 8, NF4 4-bit loading, gradient checkpointing, and a paged 8-bit optimizer. If the smoke run reports an out-of-memory error, rerun with `--max-seq-length 1024` before changing the model or quantization target.

Browser scenarios cover unavailable runtimes, profiles, headless and visible
operation, forms, tabs, dialogs, history, uploads, downloads, crashes, and
untrusted page content. They teach that the four Browser-tab rows override all
three global modes: open/history default to `alwaysAllow`, upload/download
default to `alwaysAsk`, and an allow-for-session grant is scoped to the complete
browser category.

Conduit remains the runtime authority: a model response never bypasses the
app's approval gate. Do not export a new Relay artifact until inference-backed
held-out evaluation reaches 100% risky-action approval compliance, at least 95%
schema-valid calls, and at least 90% exact-tool selection. The offline
`evaluate` command checks corpus structure only and deliberately does not claim
those model metrics.
