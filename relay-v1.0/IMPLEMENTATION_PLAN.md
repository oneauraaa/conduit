# Relay v1.0 Fine-Tuning Implementation Plan

> **For agentic workers:** Execute this plan inline in the current task. Each step is independently verifiable before continuing.

**Goal:** Build a repeatable Unsloth QLoRA pipeline that trains Relay v1.0 on Conduit tool-call and Auto Mode approval behavior, then exports Q4_K_M GGUF.

**Architecture:** A self-contained Python CLI under `relay-v1.0` owns schema validation, deterministic dataset generation, Unsloth smoke/training, export, and held-out evaluation. The canonical MCP schema is data, not Rust parsing logic, so future tool additions can be updated in one JSON file and checked before training.

**Tech Stack:** Python 3.11+, Unsloth 2026.8.18, Transformers 5.x, PEFT/TRL through Unsloth, Hugging Face safetensors, llama.cpp fallback converter, JSONL.

---

### Task 1: Define local artifacts and ignore generated weights

**Files:**
- Create: `relay-v1.0/.gitignore`
- Create: `relay-v1.0/config.json`
- Create: `relay-v1.0/README.md`

- [ ] Add `.gitignore` rules for `*.gguf`, `output/`, `cache/`, `__pycache__/`, and local model caches so the 5.78 GB reference file is never committed.
- [ ] Add JSON configuration with model ID, reference GGUF path, output paths, seed, 2,048 max sequence length, one-example micro-batch, eight-step gradient accumulation, two epochs, 2e-4 learning rate, LoRA rank 16, alpha 32, dropout 0.05, and 90/10 split.
- [ ] Document Studio/CLI setup, smoke and full commands, required disk space, expected VRAM, and that BF16 safetensors—not the existing GGUF—are the training input.
- [ ] Validate JSON parsing and confirm `git status` does not stage the reference GGUF.

### Task 2: Encode the real Conduit MCP tool surface

**Files:**
- Create: `relay-v1.0/conduit_tools.json`
- Test: `relay-v1.0/train_relay.py --prepare --validate-only`

- [ ] Encode all 24 current tools from `src-tauri/src/mcp/tools.rs`: names, descriptions, JSON-schema arguments, required fields, group, and catalog `risky` flag.
- [ ] Mark `quit_app`, `clipboard_write`, and `run_shell` as approval-gated by default; leave inspection and ordinary UI tools non-risky, matching the Rust catalog.
- [ ] Keep descriptions platform-neutral and state that runtime handshake data determines platform-specific capabilities.
- [ ] Make the validator reject duplicate names, unknown required fields, missing argument schemas, and a schema/tool mismatch.

### Task 3: Create deterministic tool-use and Auto Mode examples

**Files:**
- Create: `relay-v1.0/data/conduit_sft.jsonl`
- Create: `relay-v1.0/data/conduit_eval.jsonl`
- Modify: `relay-v1.0/train_relay.py`

- [ ] Add canonical conversation records with `messages`, `tools`, and an internal `policy` metadata field.
- [ ] Cover screen inspection, display listing, accessibility lookup followed by click, DuckDuckGo search, keybind discovery, safe shell inspection, multi-step window workflows, and Auto Mode approval before risky actions.
- [ ] Represent assistant calls in Ornith XML and tool responses with `tool` role content; include no post-call prose in the call turn.
- [ ] Include Hyprland examples where the assistant reports that RemoteDesktop portal is unavailable and uses the provided tool route rather than inventing a screen-share prompt.
- [ ] Split with a fixed seed, preserve at least one example of every tool and every risky approval case in evaluation, and validate all arguments against the schema.

### Task 4: Implement environment checks and Unsloth smoke mode

**Files:**
- Modify: `relay-v1.0/train_relay.py`

- [ ] Print Unsloth, Transformers, PyTorch, CUDA, GPU, and free-memory information.
- [ ] Require `transformers>=5.2` for the Qwen3.5 architecture, detect missing CUDA, and fail with actionable commands rather than a stack trace.
- [ ] Resolve the HF model ID or local safetensors directory, never treating the GGUF as the QLoRA input.
- [ ] Load the tokenizer and model with Unsloth 4-bit settings and apply language-only LoRA settings without image batches.
- [ ] Run a bounded one-to-three-step dry/smoke path and save it under a temporary directory; do not overwrite a completed adapter.

### Task 5: Implement full QLoRA training

**Files:**
- Modify: `relay-v1.0/train_relay.py`

- [ ] Use Unsloth’s supported Python model loader, `load_in_4bit=True`, NF4 double quantization, BF16/FP16 compute fallback, gradient checkpointing, paged 8-bit AdamW, and deterministic seed.
- [ ] Apply LoRA to language attention and MLP projections, with vision tuning disabled.
- [ ] Train only assistant/tool-call completion text when the installed TRL version supports completion-only loss; otherwise document and use the complete rendered conversation while preserving the same examples.
- [ ] Save adapter, tokenizer, resolved config, and a training summary under `relay-v1.0/output/adapter`.
- [ ] Check for OOM and report lowering sequence length before changing quantization or model family.

### Task 6: Merge, quantize, and verify Relay GGUF

**Files:**
- Modify: `relay-v1.0/train_relay.py`
- Modify: `relay-v1.0/README.md`

- [ ] Merge the adapter into a 16-bit Hugging Face checkpoint under `output/merged`.
- [ ] Export a BF16 GGUF, then run Q4_K_M quantization using Unsloth’s exporter.
- [ ] If Qwen3.5 conversion fails, call the bundled llama.cpp converter and quantizer with equivalent paths and report the exact failing stage.
- [ ] Verify the final file is non-empty, parseable by llama.cpp/Unsloth, and approximately the expected 5–7 GB range; never replace the reference GGUF.

### Task 7: Add held-out evaluation and pure-Python tests

**Files:**
- Modify: `relay-v1.0/train_relay.py`
- Create: `relay-v1.0/test_train_relay.py`

- [ ] Test schema uniqueness, deterministic split, required arguments, XML parsing, tool-name validity, and Auto Mode approval records without importing CUDA libraries.
- [ ] Score held-out output for exact tool name, required argument presence, XML parseability, and approval-before-risky-call behavior.
- [ ] Write `output/evaluation.json` with counts and failures rather than a single accuracy number.
- [ ] Run the pure-Python tests, `prepare --validate-only`, smoke loading, and export verification before claiming completion.

### Task 8: Final review and commit

**Files:**
- Modify: `relay-v1.0/README.md` if validation reveals changed commands.

- [ ] Run `python -m compileall relay-v1.0`, the pure-Python test suite, dataset preparation, and the Unsloth smoke command.
- [ ] Review `git diff --stat`, confirm no weights/caches are staged, and record the exact commands and versions in the final summary.
- [ ] Commit only source, schemas, JSONL, config, README, tests, and plan/spec files.
