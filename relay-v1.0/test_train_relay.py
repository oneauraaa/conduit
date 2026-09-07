import copy
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("train_relay", ROOT / "train_relay.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_tool_catalog_matches_conduit_surface():
    tools = module.load_tools()
    names = {tool["name"] for tool in tools}
    assert len(names) == 24
    assert {"run_shell", "quit_app", "clipboard_write"}.issubset(names)
    assert all(isinstance(tool["parameters"], dict) for tool in tools)


def test_generated_records_validate_and_cover_every_tool():
    tools = module.load_tools()
    by_name = module.tool_map(tools)
    records = module.build_records(tools)
    for record in records:
        module.validate_record(record, by_name)
    seen = {
        call["function"]["name"]
        for record in records
        for message in record["messages"]
        for call in message.get("tool_calls", [])
    }
    assert seen == set(by_name)


def test_xml_round_trip_preserves_nested_values():
    args = {"key": "s", "modifiers": ["ctrl", "shift"]}
    xml = module.render_tool_call_xml("key_press", args)
    name, parsed = module.parse_tool_call_xml(xml)
    assert name == "key_press"
    assert parsed == args


def test_auto_risky_call_requires_approval():
    tools = module.load_tools()
    by_name = module.tool_map(tools)
    records = module.build_records(tools)
    candidate = next(record for record in records if record["policy"] == "approval")
    broken = copy.deepcopy(candidate)
    broken["policy"] = "direct"
    try:
        module.validate_record(broken, by_name)
    except ValueError as exc:
        assert "risky Auto Mode" in str(exc)
    else:
        raise AssertionError("a risky Auto Mode call without approval was accepted")


def test_split_is_deterministic_and_keeps_policy_cases():
    tools = module.load_tools()
    by_name = module.tool_map(tools)
    records = module.build_records(tools)
    first = module.split_records(records, 3407, 0.1, by_name)
    second = module.split_records(records, 3407, 0.1, by_name)
    assert first == second
    assert sum(item.get("policy") == "approval" for item in first[1]) == 4


def test_written_jsonl_is_valid_after_prepare():
    for path in [ROOT / "data/conduit_sft.jsonl", ROOT / "data/conduit_eval.jsonl"]:
        if not path.exists():
            continue
        with path.open(encoding="utf-8") as handle:
            rows = [json.loads(line) for line in handle if line.strip()]
        assert rows
        assert all(row["messages"][0]["role"] == "system" for row in rows)
