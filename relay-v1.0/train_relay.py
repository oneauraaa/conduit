#!/usr/bin/env python3
"""Prepare, train, export, and evaluate Relay v1.0.

The default dataset is text-only because it teaches Conduit tool selection and
Auto Mode policy. Training therefore uses Unsloth's text-only Qwen3.5 path,
which fits a 16 GB GPU more reliably. A future dataset containing image items
can opt into the full vision processor with --with-vision-data.
"""
from __future__ import annotations

import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent
CONFIG_PATH = ROOT / "config.json"
TOOLS_PATH = ROOT / "conduit_tools.json"
DATA_DIR = ROOT / "data"
TRAIN_PATH = DATA_DIR / "conduit_sft.jsonl"
EVAL_PATH = DATA_DIR / "conduit_eval.jsonl"
OUTPUT_DIR = ROOT / "output"

# Dataset code stays CPU-only and can be used without the Unsloth environment.
from relay_dataset import (
    load_json, load_tools, tool_map, build_records, validate_args, validate_record,
    render_tool_call_xml, parse_tool_call_xml, read_jsonl, write_jsonl,
    split_records, audit_split, statistics, completion_rows,
)


def load_config():
    return load_json(CONFIG_PATH)


def prepare(args):
    config = load_config()
    by_name = tool_map(load_tools())
    records = build_records(list(by_name.values()))
    for record in records:
        validate_record(record, by_name)
    train, evaluation = split_records(records, config['seed'], config['eval_split'], by_name)
    manifest = {
        'dataset_version': 2,
        'provenance': 'curated synthetic conversations; no recorded user sessions',
        'seed': config['seed'],
        'requested_eval_fraction': config['eval_split'],
        'actual_eval_fraction': round(len(evaluation)/len(records), 4),
        'split_strategy': 'whole scenario families; coverage in both splits takes precedence over fraction',
        'total_records': len(records), 'total_families': len({r['group'] for r in records}),
        'train': statistics(train), 'evaluation': statistics(evaluation),
        'shared_families': [],
        'model_performance_measured': False,
    }
    if not args.validate_only:
        write_jsonl(TRAIN_PATH,train)
        write_jsonl(EVAL_PATH,evaluation)
        (DATA_DIR/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
    print(f"Validated {len(records)} conversations / {manifest['total_families']} families: "
          f"{len(train)} train, {len(evaluation)} evaluation. All {len(by_name)} tools in both splits.")


def print_environment() -> dict[str, Any]:
    try:
        import torch
        import transformers
        import trl
        import peft
        import bitsandbytes
        import unsloth
    except ImportError as exc:
        raise RuntimeError(
            "The Unsloth environment is incomplete. Run this script with the Studio interpreter, "
            "for example: $HOME/.unsloth/studio/unsloth_studio/bin/python relay-v1.0/train_relay.py smoke"
        ) from exc
    if not torch.cuda.is_available():
        raise RuntimeError("CUDA is not available; QLoRA training requires the NVIDIA GPU environment")
    info = {
        "unsloth": getattr(unsloth, "__version__", "unknown"),
        "transformers": transformers.__version__,
        "trl": trl.__version__,
        "peft": peft.__version__,
        "bitsandbytes": bitsandbytes.__version__,
        "torch": torch.__version__,
        "cuda": torch.version.cuda,
        "gpu": torch.cuda.get_device_name(0),
        "vram_gb": round(torch.cuda.get_device_properties(0).total_memory / 1024**3, 2),
        "bf16": bool(torch.cuda.is_bf16_supported()),
    }
    print(json.dumps(info, indent=2))
    if tuple(int(part) for part in transformers.__version__.split(".")[:2]) < (5, 2):
        raise RuntimeError("Qwen3.5 requires Transformers >= 5.2")
    return info


def load_training_stack(config: dict[str, Any], *, smoke: bool = False, with_vision_data: bool = False):
    print_environment()
    # Import Unsloth before TRL/Transformers model classes so its patches are active.
    from unsloth import FastLanguageModel
    import torch

    model_id = config["model_id"]
    cache_dir = ROOT / config["model_cache_dir"]
    cache_dir.mkdir(parents=True, exist_ok=True)
    max_length = int(config["max_seq_length"])
    dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
    if with_vision_data:
        raise RuntimeError(
            "The current Relay corpus is text-only. Add image-bearing records and a vision collator before "
            "using --with-vision-data; this avoids silently training a VLM without pixel inputs."
        )
    print(f"loading {model_id} in 4-bit NF4 text-only mode; max sequence length={max_length}")
    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=model_id,
        max_seq_length=max_length,
        dtype=dtype,
        load_in_4bit=True,
        use_gradient_checkpointing="unsloth",
        cache_dir=str(cache_dir),
        text_only=True,
        trust_remote_code=False,
        random_state=int(config["seed"]),
    )
    model = FastLanguageModel.get_peft_model(
        model,
        r=int(config["lora_r"]),
        target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
        lora_alpha=int(config["lora_alpha"]),
        lora_dropout=float(config["lora_dropout"]),
        bias="none",
        use_gradient_checkpointing="unsloth",
        random_state=int(config["seed"]),
        max_seq_length=max_length,
        use_rslora=False,
    )
    return model, tokenizer, torch


def load_dataset_for_training(path: Path, tokenizer, max_length):
    from datasets import Dataset
    records = read_jsonl(path)
    by_name = tool_map(load_tools())
    for record in records:
        validate_record(record,by_name)
    if not records:
        raise ValueError(f'empty dataset: {path}')
    return Dataset.from_list(completion_rows(records,tokenizer,max_length))


def trainer_for(config: dict[str, Any], model: Any, tokenizer: Any, train_dataset: Any, eval_dataset: Any, *, smoke: bool):
    from trl import SFTConfig, SFTTrainer
    import torch

    output = ROOT / config["output_dir"] / ("smoke" if smoke else "adapter")
    output.mkdir(parents=True, exist_ok=True)
    bf16 = torch.cuda.is_bf16_supported()
    max_steps = 2 if smoke else -1
    sft_args = SFTConfig(
        output_dir=str(output),
        per_device_train_batch_size=int(config["per_device_train_batch_size"]),
        per_device_eval_batch_size=1,
        gradient_accumulation_steps=int(config["gradient_accumulation_steps"]),
        num_train_epochs=float(config["num_train_epochs"]),
        max_steps=max_steps,
        learning_rate=float(config["learning_rate"]),
        warmup_ratio=float(config["warmup_ratio"]),
        weight_decay=float(config["weight_decay"]),
        optim="paged_adamw_8bit",
        bf16=bf16,
        fp16=not bf16,
        gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
        logging_steps=1,
        save_strategy="no" if smoke else "epoch",
        eval_strategy="no" if smoke else "epoch",
        report_to="none",
        seed=int(config["seed"]),
        data_seed=int(config["seed"]),
        max_length=int(config["max_seq_length"]),
        packing=False,
        completion_only_loss=True,
        assistant_only_loss=False,
        remove_unused_columns=False,
        dataset_num_proc=1,
        dataset_kwargs={'skip_prepare_dataset': True},
    )
    return SFTTrainer(
        model=model,
        args=sft_args,
        train_dataset=train_dataset,
        eval_dataset=None if smoke else eval_dataset,
        processing_class=tokenizer,
    )


def train(args: argparse.Namespace, *, smoke: bool) -> None:
    config = load_config()
    if not TRAIN_PATH.exists() or not EVAL_PATH.exists():
        raise ValueError('Prepared datasets are missing. Run prepare before training.')
    audit_split(read_jsonl(TRAIN_PATH), read_jsonl(EVAL_PATH), tool_map(load_tools()))
    model, tokenizer, torch = load_training_stack(config, smoke=smoke, with_vision_data=args.with_vision_data)
    train_dataset = load_dataset_for_training(TRAIN_PATH, tokenizer, int(config["max_seq_length"]))
    eval_dataset = load_dataset_for_training(EVAL_PATH, tokenizer, int(config["max_seq_length"]))
    trainer = trainer_for(config, model, tokenizer, train_dataset, eval_dataset, smoke=smoke)
    print(f"starting {'smoke' if smoke else 'full'} training")
    result = trainer.train()
    destination = ROOT / config["output_dir"] / ("smoke" if smoke else "adapter")
    if smoke:
        trainer.save_model(str(destination))
    else:
        model.save_pretrained(str(destination))
        tokenizer.save_pretrained(str(destination))
    summary = {
        "mode": "smoke" if smoke else "train",
        "global_step": int(result.global_step),
        "training_loss": float(result.training_loss) if result.training_loss is not None else None,
        "output": str(destination),
    }
    (destination / "training_summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2))


def find_gguf(path: Path) -> Path | None:
    if path.is_file() and path.suffix == ".gguf":
        return path
    candidates = sorted(path.glob("*.gguf")) if path.exists() else []
    return candidates[0] if candidates else None


def fallback_gguf(merged: Path, gguf_dir: Path, final: Path) -> None:
    """Use the llama.cpp tools bundled by Unsloth when its wrapper cannot convert Qwen3.5."""
    converter_candidates = [
        Path.home() / ".unsloth/llama.cpp/convert_hf_to_gguf.py",
        Path.home() / ".unsloth/llama.cpp/unsloth_convert_hf_to_gguf.py",
    ]
    converter = next((path for path in converter_candidates if path.exists()), None)
    quantizer = shutil.which("llama-quantize") or str(Path.home() / ".unsloth/llama.cpp/build/bin/llama-quantize")
    if converter is None:
        raise RuntimeError("Unsloth export failed and no bundled convert_hf_to_gguf.py was found")
    if not Path(quantizer).exists():
        raise RuntimeError("Unsloth export failed and no bundled llama-quantize binary was found")
    bf16 = gguf_dir / "Ornith-1.5-9B-Relay-v1.0-BF16.gguf"
    convert = subprocess.run(
        [sys.executable, str(converter), str(merged), "--outfile", str(bf16), "--outtype", "bf16"],
        text=True,
        capture_output=True,
    )
    if convert.returncode != 0:
        raise RuntimeError(f"llama.cpp HF conversion failed:\n{convert.stdout}\n{convert.stderr}")
    quantize = subprocess.run(
        [str(quantizer), str(bf16), str(final), "Q4_K_M"],
        text=True,
        capture_output=True,
    )
    if quantize.returncode != 0:
        raise RuntimeError(f"llama-quantize failed:\n{quantize.stdout}\n{quantize.stderr}")


def export(args: argparse.Namespace) -> None:
    config = load_config()
    adapter = ROOT / config["output_dir"] / "adapter"
    if not adapter.exists():
        raise RuntimeError(f"adapter not found at {adapter}; run the train command first")
    print_environment()
    from unsloth import FastLanguageModel
    from peft import PeftModel
    import torch

    cache_dir = ROOT / config["model_cache_dir"]
    dtype = torch.bfloat16 if torch.cuda.is_bf16_supported() else torch.float16
    model, tokenizer = FastLanguageModel.from_pretrained(
        model_name=config["model_id"],
        max_seq_length=int(config["max_seq_length"]),
        dtype=dtype,
        load_in_4bit=True,
        use_gradient_checkpointing="unsloth",
        cache_dir=str(cache_dir),
        text_only=True,
        trust_remote_code=False,
    )
    model = PeftModel.from_pretrained(model, str(adapter), is_trainable=False)
    model.eval()
    output = ROOT / config["output_dir"]
    merged = output / "merged"
    merged.mkdir(parents=True, exist_ok=True)
    gguf_dir = output / "gguf"
    gguf_dir.mkdir(parents=True, exist_ok=True)
    print("merging LoRA adapter into 16-bit Hugging Face weights")
    try:
        model.save_pretrained_merged(str(merged), tokenizer, save_method="merged_16bit")
    except AttributeError:
        merged_model = model.merge_and_unload()
        merged_model.save_pretrained(str(merged), safe_serialization=True, max_shard_size="5GB")
        tokenizer.save_pretrained(str(merged))
    final = output / "Ornith-1.5-9B-Relay-v1.0-Q4_K_M.gguf"
    print("exporting Q4_K_M GGUF")
    try:
        model.save_pretrained_gguf(str(gguf_dir), tokenizer, quantization_method="q4_k_m", merge_is_disposable=False)
        candidates = sorted(gguf_dir.glob("*.gguf"))
        if not candidates:
            raise RuntimeError(f"Unsloth export completed without a GGUF in {gguf_dir}")
        shutil.copy2(candidates[0], final)
    except Exception as exc:
        print(f"Unsloth GGUF export failed ({exc}); trying bundled llama.cpp tools")
        fallback_gguf(merged, gguf_dir, final)
    if not final.exists() or final.stat().st_size == 0:
        raise RuntimeError("exported GGUF is empty")
    print(json.dumps({"gguf": str(final), "bytes": final.stat().st_size}, indent=2))


def template_check(args):
    """Offline: use cached tokenizer assets; never load weights or start a trainer."""
    config = load_config()
    from transformers import AutoTokenizer
    tokenizer = AutoTokenizer.from_pretrained(
        config['model_id'], cache_dir=str(ROOT/config['model_cache_dir']),
        local_files_only=True, trust_remote_code=False,
    )
    report = {'kind':'offline_template_audit','model_performance_measured':False,'splits':{}}
    for name,path in [('train',TRAIN_PATH),('evaluation',EVAL_PATH)]:
        rows = completion_rows(read_jsonl(path),tokenizer,int(config['max_seq_length']))
        report['splits'][name] = {
            'assistant_targets':len(rows),
            'max_tokens':max(len(r['input_ids']) for r in rows),
            'supervised_tokens':sum(sum(r['completion_mask']) for r in rows),
        }
    print(json.dumps(report,indent=2))


def evaluate(args):
    """Structural corpus audit, not a model inference score."""
    by_name = tool_map(load_tools())
    train,evaluation = read_jsonl(TRAIN_PATH),read_jsonl(EVAL_PATH)
    audit_split(train,evaluation,by_name)
    report = {'kind':'dataset_audit','model_performance_measured':False,
              'train':statistics(train),'evaluation':statistics(evaluation),'valid':True}
    output = ROOT/load_config()['output_dir']
    output.mkdir(parents=True,exist_ok=True)
    (output/'dataset_audit.json').write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,indent=2))


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prepare_parser = sub.add_parser("prepare", help="write and validate deterministic JSONL data")
    prepare_parser.add_argument("--validate-only", action="store_true")
    smoke_parser = sub.add_parser("smoke", help="load Unsloth and run two bounded optimizer steps")
    smoke_parser.add_argument("--with-vision-data", action="store_true")
    train_parser = sub.add_parser("train", help="run the full QLoRA training job")
    train_parser.add_argument("--with-vision-data", action="store_true")
    sub.add_parser("export", help="merge the adapter and export Q4_K_M GGUF")
    sub.add_parser("template-check", help="render all records with Ornith's tokenizer template")
    sub.add_parser("evaluate", help="audit dataset structure and split coverage (no model inference)")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command == "prepare":
        prepare(args)
    elif args.command == "smoke":
        train(args, smoke=True)
    elif args.command == "train":
        train(args, smoke=False)
    elif args.command == "export":
        export(args)
    elif args.command == "template-check":
        template_check(args)
    elif args.command == "evaluate":
        evaluate(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
