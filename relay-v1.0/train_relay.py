#!/usr/bin/env python3
"""Prepare, train, export, and evaluate Relay v1.0.

The default dataset is text-only because it teaches Conduit tool selection and
Auto Mode policy. Training therefore uses Unsloth's text-only Qwen3.5 path,
which fits a 16 GB GPU more reliably. A future dataset containing image items
can opt into the full vision processor with --with-vision-data.
"""
from __future__ import annotations

import argparse
import copy
import json
import random
import re
import shutil
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Any, Iterable

ROOT = Path(__file__).resolve().parent
CONFIG_PATH = ROOT / "config.json"
TOOLS_PATH = ROOT / "conduit_tools.json"
DATA_DIR = ROOT / "data"
TRAIN_PATH = DATA_DIR / "conduit_sft.jsonl"
EVAL_PATH = DATA_DIR / "conduit_eval.jsonl"
OUTPUT_DIR = ROOT / "output"

TOOL_CALL_RE = re.compile(
    r"<tool_call>\s*<function=(?P<name>[^>\s]+)>\s*(?P<body>.*?)\s*</function>\s*</tool_call>",
    re.DOTALL,
)
PARAM_RE = re.compile(r"<parameter=(?P<name>[^>]+)>\s*(?P<value>.*?)\s*</parameter>", re.DOTALL)


def load_json(path: Path) -> Any:
    with path.open(encoding="utf-8") as handle:
        return json.load(handle)


def load_config() -> dict[str, Any]:
    return load_json(CONFIG_PATH)


def load_tools() -> list[dict[str, Any]]:
    tools = load_json(TOOLS_PATH)
    if not isinstance(tools, list):
        raise ValueError("conduit_tools.json must contain a JSON array")
    return tools


def tool_map(tools: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    result: dict[str, dict[str, Any]] = {}
    for tool in tools:
        name = tool.get("name")
        if not isinstance(name, str) or not name:
            raise ValueError("every tool needs a non-empty name")
        if name in result:
            raise ValueError(f"duplicate tool name: {name}")
        if not isinstance(tool.get("parameters"), dict):
            raise ValueError(f"{name}: parameters must be an object schema")
        required = tool["parameters"].get("required", [])
        properties = tool["parameters"].get("properties", {})
        if not isinstance(required, list) or not isinstance(properties, dict):
            raise ValueError(f"{name}: invalid required/properties schema")
        unknown_required = set(required) - set(properties)
        if unknown_required:
            raise ValueError(f"{name}: required fields missing from properties: {sorted(unknown_required)}")
        result[name] = tool
    if len(result) != 24:
        raise ValueError(f"expected 24 Conduit tools, found {len(result)}")
    return result


def model_tool(tool: dict[str, Any]) -> dict[str, Any]:
    """Convert the canonical Conduit record to Ornith's OpenAI-style tool schema."""
    return {
        "type": "function",
        "function": {
            "name": tool["name"],
            "description": tool["description"],
            "parameters": tool["parameters"],
        },
    }


def model_tools(names: Iterable[str], tools_by_name: dict[str, dict[str, Any]]) -> list[dict[str, Any]]:
    unique = list(dict.fromkeys(names))
    return [model_tool(tools_by_name[name]) for name in unique]


def system_prompt(mode: str) -> str:
    mode = mode.lower()
    mode_name = {"auto": "Auto Mode", "manual": "Manual Mode", "full": "Full Access"}.get(mode, mode)
    return textwrap.dedent(
        f"""
        You are Relay, the tool-using assistant connected to Conduit.

        Current access mode: {mode_name}.
        Conduit is driving a Linux Wayland desktop. On Hyprland, use the tools that are actually
        available. Do not invent a GNOME/KDE screen-share prompt or claim that the RemoteDesktop
        portal exists when the server says it is unavailable. Never claim an action happened until
        a tool response confirms it.

        In Auto Mode, ask the user for approval before a potentially destructive or state-changing
        action. This includes Conduit's risky tools (run_shell, quit_app, and clipboard_write) and
        shell commands that could delete data, alter the system, expose secrets, or overwrite work.
        Put the approval request in its own assistant turn and wait for the user's answer before
        emitting a tool call. Read-only inspection and ordinary navigation can proceed when asked.
        Conduit remains the final runtime authority and may still show its own approval card.

        Prefer read_screen_text or find_element before screenshot when exact text or controls are
        enough. Use the tool schemas supplied with this conversation and include every required
        argument in a tool call.
        """
    ).strip()


def json_text(value: Any) -> str:
    if isinstance(value, str):
        return value
    return json.dumps(value, ensure_ascii=False, sort_keys=True)


def call(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    return {"function": {"name": name, "arguments": arguments}}


def make_record(
    record_id: str,
    mode: str,
    user: str,
    tools_by_name: dict[str, dict[str, Any]],
    *,
    calls: list[tuple[str, dict[str, Any], Any]] | None = None,
    final: str | None = None,
    approval_request: str | None = None,
    available: list[str] | None = None,
    policy: str = "direct",
) -> dict[str, Any]:
    calls = calls or []
    call_names = [name for name, _, _ in calls]
    available_names = available or call_names
    if not available_names:
        available_names = ["read_screen_text", "screenshot", "list_displays"]
    messages: list[dict[str, Any]] = [
        {"role": "system", "content": system_prompt(mode)},
        {"role": "user", "content": user},
    ]
    if approval_request:
        messages.append({"role": "assistant", "content": approval_request})
        messages.append({"role": "user", "content": "Approved. Proceed with that action."})
    if calls:
        messages.append(
            {
                "role": "assistant",
                "content": "",
                "tool_calls": [call(name, arguments) for name, arguments, _ in calls],
            }
        )
        for name, _, result in calls:
            messages.append({"role": "tool", "content": json_text(result), "name": name})
    if final:
        messages.append({"role": "assistant", "content": final})
    return {
        "id": record_id,
        "mode": mode,
        "policy": policy,
        "tools": model_tools(available_names, tools_by_name),
        "messages": messages,
    }


def build_records(tools: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_name = tool_map(tools)
    records: list[dict[str, Any]] = []

    def direct(
        record_id: str,
        mode: str,
        user: str,
        name: str,
        args: dict[str, Any],
        result: Any,
        final: str,
        available: list[str] | None = None,
        policy: str = "direct",
    ) -> None:
        records.append(
            make_record(
                record_id,
                mode,
                user,
                by_name,
                calls=[(name, args, result)],
                final=final,
                available=available,
                policy=policy,
            )
        )

    # One explicit example for each current Conduit tool.
    direct("displays-01", "auto", "How many monitors are connected?", "list_displays", {}, [{"index": 0, "x": 0, "y": 0, "width": 2560, "height": 1440}], "There is one connected display, 2560 by 1440.")
    direct("screenshot-01", "manual", "Show me the current screen.", "screenshot", {"display": 0, "scale": 0.5}, "display 0, 2560x1440 of coordinate space captured at 1280x720 pixels", "The current screen has been captured.")
    direct("cursor-01", "manual", "Move the cursor to the center of the display.", "move_cursor", {"x": 1280, "y": 720}, "cursor at 1280, 720", "The cursor is at the center of the display.")
    direct("click-01", "manual", "Click the Save button at 840, 612.", "click", {"x": 840, "y": 612, "button": "left", "count": 1}, "clicked at 840, 612", "The Save button was clicked.")
    direct("drag-01", "manual", "Drag the slider from 400, 500 to 720, 500.", "drag", {"from_x": 400, "from_y": 500, "to_x": 720, "to_y": 500, "button": "left"}, "dragged to 720, 500", "The slider was dragged to the requested position.")
    direct("scroll-01", "manual", "Scroll up over the page.", "scroll", {"dy": 640}, "scrolled dx 0, dy 640", "The page was scrolled up.")
    direct("type-01", "manual", "Type `hello from Relay` into the focused field.", "type_text", {"text": "hello from Relay"}, "typed 16 characters", "The text was entered.")
    direct("key-01", "manual", "Copy the selected text.", "key_press", {"key": "c", "modifiers": ["cmd"]}, "pressed cmd+c", "The copy shortcut was pressed.")
    direct("position-01", "auto", "Where is the cursor now?", "get_cursor_position", {}, {"x": 920, "y": 488}, "The cursor is at (920, 488).")
    direct("windows-01", "auto", "List the open windows.", "list_windows", {}, [{"window_id": 41, "title": "Browser", "app": "Firefox", "x": 0, "y": 0, "width": 1280, "height": 900}], "The open windows are Firefox (Browser), window 41.")
    direct("focus-01", "manual", "Bring window 41 to the front.", "focus_window", {"window_id": 41}, "focused window 41", "Window 41 is focused.")
    direct("bounds-01", "manual", "Move window 41 to the top-left and make it 1200 by 800.", "set_window_bounds", {"window_id": 41, "x": 0, "y": 0, "width": 1200, "height": 800}, "window moved", "Window 41 was moved and resized.")
    direct("apps-01", "auto", "Which applications are running?", "list_apps", {}, [{"name": "Firefox", "pid": 4101}, {"name": "Alacritty", "pid": 4202}], "Firefox and Alacritty are running.")
    direct("open-01", "manual", "Open Firefox.", "open_app", {"name": "Firefox"}, "opened Firefox", "Firefox is open.")
    direct("quit-01-manual", "manual", "Close Calculator; I have saved my work.", "quit_app", {"name": "Calculator"}, "asked Calculator to quit", "Calculator was asked to quit.", policy="explicit")
    direct("ax-01", "auto", "Read the visible controls in the frontmost app.", "read_screen_text", {}, [{"role": "button", "name": "Save", "centerX": 840, "centerY": 612}], "The frontmost app exposes a Save button at (840, 612).")
    direct("find-01", "auto", "Find the Settings button.", "find_element", {"query": "Settings"}, [{"name": "Settings", "centerX": 320, "centerY": 240}], "The Settings control is at (320, 240).")
    direct("search-01", "auto", "Search DuckDuckGo for the latest Unsloth Qwen3.5 guide.", "web_search", {"query": "Unsloth Qwen3.5 fine tuning guide", "max_results": 5}, [{"title": "Qwen3.5 fine-tuning guide", "url": "https://unsloth.ai/docs/models/qwen3.5/fine-tune", "snippet": "Fine-tune Qwen3.5 with Unsloth."}], "DuckDuckGo returned a Qwen3.5 fine-tuning guide.")
    direct("clipboard-read-01", "auto", "What text is currently copied?", "clipboard_read", {}, "release candidate", "The clipboard contains `release candidate`.")
    direct("shell-manual-01", "manual", "Run `git status --short` in the repository.", "run_shell", {"command": "git status --short", "timeout_seconds": 30}, "exit 0\n\n M relay-v1.0/train_relay.py", "The repository has one modified training script.", policy="explicit")
    direct("wait-01", "auto", "Wait half a second for the window to appear.", "wait", {"milliseconds": 500}, "waited 500ms", "The wait is complete.")
    direct("notify-01", "manual", "Notify me that the export finished.", "notify", {"title": "Relay", "body": "The export finished."}, "notification posted", "The notification was posted.")
    direct("keybinds-01", "auto", "Which keyboard shortcuts does Hyprland publish?", "list_keybinds", {}, [{"available": True, "bindings": [{"key": "Return", "modifiers": ["SUPER"], "action": "exec terminal"}]}], "Hyprland publishes a Super+Return shortcut for opening a terminal.")
    direct("clipboard-write-manual", "manual", "Set the clipboard to `approved text`.", "clipboard_write", {"text": "approved text"}, "clipboard updated", "The clipboard was updated.", policy="explicit")

    # Auto Mode approval examples: the approval turn is separate from the tool call.
    records.append(
        make_record(
            "shell-approval-rm",
            "auto",
            "Remove the old build directory with `rm -rf build`.",
            by_name,
            approval_request="That command would recursively delete files. Auto Mode requires your approval before I run it. Should I proceed?",
            calls=[("run_shell", {"command": "rm -rf build", "timeout_seconds": 30}, "exit 0\n\n(no output)")],
            final="The build directory was removed.",
            policy="approval",
        )
    )
    records.append(
        make_record(
            "quit-approval",
            "auto",
            "Close the unsaved editor window.",
            by_name,
            approval_request="Closing the editor may discard unsaved work. Auto Mode requires your approval. Should I ask the application to quit?",
            calls=[("quit_app", {"name": "Text Editor"}, "asked Text Editor to quit")],
            final="The Text Editor was asked to quit.",
            policy="approval",
        )
    )
    records.append(
        make_record(
            "clipboard-approval",
            "auto",
            "Replace the clipboard with the generated deployment token.",
            by_name,
            approval_request="This will overwrite the text currently in your clipboard. Auto Mode requires approval before I do that. Proceed?",
            calls=[("clipboard_write", {"text": "deploy-token-7f31"}, "clipboard updated")],
            final="The clipboard now contains the deployment token.",
            policy="approval",
        )
    )
    records.append(
        make_record(
            "shell-approval-reset",
            "auto",
            "Reset the repository with `git reset --hard HEAD`.",
            by_name,
            approval_request="That command discards uncommitted changes. I need your approval before running it in Auto Mode. Proceed?",
            calls=[("run_shell", {"command": "git reset --hard HEAD", "timeout_seconds": 30}, "exit 0\n\nHEAD is now at 58169d1")],
            final="The repository was reset to HEAD.",
            policy="approval",
        )
    )

    # Multi-step workflows teach the model to use observations rather than guess.
    records.append(
        make_record(
            "workflow-find-click",
            "auto",
            "Open the Settings page in the frontmost app.",
            by_name,
            calls=[
                ("find_element", {"query": "Settings"}, [{"name": "Settings", "centerX": 320, "centerY": 240}]),
                ("click", {"x": 320, "y": 240, "button": "left", "count": 1}, "clicked at 320, 240"),
            ],
            final="I found and clicked Settings.",
            available=["find_element", "click"],
            policy="sequence",
        )
    )
    records.append(
        make_record(
            "workflow-window-focus",
            "auto",
            "Focus the browser window, but inspect the window list first.",
            by_name,
            calls=[
                ("list_windows", {}, [{"window_id": 41, "title": "Browser", "app": "Firefox"}]),
                ("focus_window", {"window_id": 41}, "focused window 41"),
            ],
            final="The Browser window is focused.",
            available=["list_windows", "focus_window"],
            policy="sequence",
        )
    )
    records.append(
        make_record(
            "workflow-keybind",
            "auto",
            "Use the user's published shortcut to open a terminal.",
            by_name,
            calls=[
                ("list_keybinds", {}, [{"available": True, "bindings": [{"key": "Return", "modifiers": ["SUPER"], "action": "exec terminal"}]}]),
                ("key_press", {"key": "return", "modifiers": ["super"]}, "pressed super+return"),
            ],
            final="I read the published binding and pressed Super+Return.",
            available=["list_keybinds", "key_press"],
            policy="sequence",
        )
    )
    records.append(
        make_record(
            "workflow-display-shot",
            "auto",
            "Inspect the displays, then capture the secondary display if one exists.",
            by_name,
            calls=[
                ("list_displays", {}, [{"index": 0, "x": 0, "y": 0, "width": 2560, "height": 1440}, {"index": 1, "x": 2560, "y": 0, "width": 1920, "height": 1080}]),
                ("screenshot", {"display": 1, "scale": 0.5}, "display 1, 1920x1080 of coordinate space captured at 960x540 pixels"),
            ],
            final="I found a secondary display and captured it.",
            available=["list_displays", "screenshot"],
            policy="sequence",
        )
    )
    records.append(
        make_record(
            "workflow-accessibility-click",
            "auto",
            "Find and click the Continue button without using a screenshot.",
            by_name,
            calls=[
                ("read_screen_text", {}, [{"role": "button", "name": "Continue", "centerX": 1100, "centerY": 760}]),
                ("click", {"x": 1100, "y": 760, "button": "left", "count": 1}, "clicked at 1100, 760"),
            ],
            final="The Continue button was found in the accessibility tree and clicked.",
            available=["read_screen_text", "click"],
            policy="sequence",
        )
    )

    # Policy answers without a tool call prevent hallucinated portal capabilities.
    records.append(
        make_record(
            "hyprland-portal",
            "auto",
            "Ask the desktop portal for a remote-desktop screenshare on Hyprland.",
            by_name,
            final="This Hyprland session does not provide the RemoteDesktop portal interface. I will use the available Conduit tools instead of claiming that a screenshare prompt exists.",
            available=["list_displays", "screenshot", "read_screen_text"],
            policy="capability",
        )
    )
    records.append(
        make_record(
            "clarify-click",
            "auto",
            "Click it.",
            by_name,
            final="Which control should I click? Please provide its label or location; I will inspect the screen before acting.",
            available=["read_screen_text", "find_element", "click"],
            policy="clarify",
        )
    )

    # Add controlled wording/argument variants so the model does not memorize one phrase.
    direct(
        "search-02", "auto", "Look up the Hyprland virtual pointer protocol on DuckDuckGo.", "web_search",
        {"query": "Hyprland virtual pointer protocol", "max_results": 3},
        [{"title": "wlr virtual pointer", "url": "https://wayland.app/protocols/wlr-virtual-pointer-unstable-v1", "snippet": "A virtual pointer protocol."}],
        "DuckDuckGo returned information about the wlr virtual pointer protocol.",
    )
    direct("screenshot-02", "manual", "Capture a smaller region of display 0.", "screenshot", {"display": 0, "region": [0, 0, 800, 600], "scale": 1.0}, "display 0, 800x600 of coordinate space captured at 800x600 pixels", "The requested region was captured.")
    direct("click-02", "manual", "Double-click the file at 500, 420.", "click", {"x": 500, "y": 420, "button": "left", "count": 2}, "clicked at 500, 420", "The file was double-clicked.")
    direct("key-02", "manual", "Open the browser address bar.", "key_press", {"key": "l", "modifiers": ["ctrl"]}, "pressed ctrl+l", "The browser address bar shortcut was pressed.")
    direct("type-02", "manual", "Enter the Japanese greeting こんにちは.", "type_text", {"text": "こんにちは"}, "typed 5 characters", "The Unicode text was entered.")
    direct("find-02", "auto", "Locate the Preferences control in Firefox.", "find_element", {"query": "Preferences", "app": "Firefox"}, [{"name": "Preferences", "centerX": 500, "centerY": 300}], "Preferences is at (500, 300).")
    direct("shell-manual-02", "manual", "Show the current directory with `pwd`.", "run_shell", {"command": "pwd", "timeout_seconds": 30}, "exit 0\n\n/home/oneaura/Documents/Coding/conduit", "The current directory is the Conduit repository.", policy="explicit")
    direct("open-02", "manual", "Launch the terminal application.", "open_app", {"name": "Alacritty"}, "opened Alacritty", "Alacritty is open.")
    direct("scroll-02", "manual", "Scroll down a little on the page.", "scroll", {"dy": -420, "x": 900, "y": 700}, "scrolled dx 0, dy -420", "The page was scrolled down.")
    direct("clipboard-read-02", "auto", "Read the clipboard before I paste it.", "clipboard_read", {}, "meeting notes", "The clipboard contains `meeting notes`.")
    direct("position-02", "auto", "Check the cursor before clicking.", "get_cursor_position", {}, {"x": 100, "y": 200}, "The cursor is at (100, 200).")
    direct("wait-02", "auto", "Give the application two seconds to settle.", "wait", {"milliseconds": 2000}, "waited 2000ms", "The application had two seconds to settle.")
    direct("notify-02", "manual", "Show a notification saying the model is ready.", "notify", {"title": "Relay", "body": "The model is ready."}, "notification posted", "The notification was posted.")
    direct("bounds-02", "manual", "Resize window 41 to 900 by 700 without moving it.", "set_window_bounds", {"window_id": 41, "x": 0, "y": 0, "width": 900, "height": 700}, "window moved", "Window 41 was resized.")

    # A second natural-language phrasing for each one-call example gives the
    # adapter more than one trigger without inventing extra tool semantics.
    seed_records = copy.deepcopy(records)
    for record in seed_records:
        if record.get("policy") not in {"direct", "explicit"}:
            continue
        if len(record.get("messages", [])) < 2 or record["messages"][1].get("role") != "user":
            continue
        variant = copy.deepcopy(record)
        variant["id"] = f"{record['id']}-variant"
        variant["messages"][1]["content"] = "Please handle this request through Conduit: " + str(record["messages"][1]["content"])
        records.append(variant)

    return records


def validate_type(value: Any, schema: dict[str, Any], path: str) -> None:
    expected = schema.get("type")
    if expected == "string" and not isinstance(value, str):
        raise ValueError(f"{path} must be a string")
    if expected == "number" and (not isinstance(value, (int, float)) or isinstance(value, bool)):
        raise ValueError(f"{path} must be a number")
    if expected == "integer" and (not isinstance(value, int) or isinstance(value, bool)):
        raise ValueError(f"{path} must be an integer")
    if expected == "array" and not isinstance(value, list):
        raise ValueError(f"{path} must be an array")
    if expected == "object" and not isinstance(value, dict):
        raise ValueError(f"{path} must be an object")
    if isinstance(schema.get("enum"), list) and value not in schema["enum"]:
        raise ValueError(f"{path} must be one of {schema['enum']}")
    if expected == "array" and "items" in schema:
        for index, item in enumerate(value):
            validate_type(item, schema["items"], f"{path}[{index}]")


def render_tool_call_xml(name: str, arguments: dict[str, Any]) -> str:
    lines = ["<tool_call>", f"<function={name}>"]
    for key, value in arguments.items():
        if isinstance(value, (dict, list)):
            rendered = json.dumps(value, ensure_ascii=False, separators=(",", ":"))
        elif isinstance(value, bool):
            rendered = "true" if value else "false"
        else:
            rendered = str(value)
        lines.extend([f"<parameter={key}>", rendered, "</parameter>"])
    lines.extend(["</function>", "</tool_call>"])
    return "\n".join(lines)


def parse_tool_call_xml(text: str) -> tuple[str, dict[str, Any]]:
    match = TOOL_CALL_RE.search(text)
    if not match:
        raise ValueError("assistant output has no complete <tool_call> XML block")
    args: dict[str, Any] = {}
    for parameter in PARAM_RE.finditer(match.group("body")):
        name = parameter.group("name").strip()
        raw = parameter.group("value").strip()
        try:
            args[name] = json.loads(raw)
        except json.JSONDecodeError:
            args[name] = raw
    return match.group("name"), args


def validate_args(tool: dict[str, Any], arguments: dict[str, Any]) -> None:
    schema = tool["parameters"]
    properties = schema.get("properties", {})
    required = schema.get("required", [])
    unknown = set(arguments) - set(properties)
    if unknown:
        raise ValueError(f"{tool['name']}: unknown arguments {sorted(unknown)}")
    missing = set(required) - set(arguments)
    if missing:
        raise ValueError(f"{tool['name']}: missing required arguments {sorted(missing)}")
    for name, value in arguments.items():
        validate_type(value, properties[name], f"{tool['name']}.{name}")


def validate_record(record: dict[str, Any], tools_by_name: dict[str, dict[str, Any]]) -> None:
    if not isinstance(record.get("messages"), list) or len(record["messages"]) < 2:
        raise ValueError(f"{record.get('id')}: messages must contain system and user turns")
    if record["messages"][0].get("role") != "system" or record["messages"][1].get("role") != "user":
        raise ValueError(f"{record.get('id')}: first turns must be system then user")
    offered = {item.get("function", {}).get("name") for item in record.get("tools", [])}
    calls_seen: list[tuple[str, dict[str, Any]]] = []
    approval_text_seen = False
    approval_user_seen = False
    for message in record["messages"]:
        if message.get("role") == "assistant" and "approval" in str(message.get("content", "")).lower():
            approval_text_seen = True
        if message.get("role") == "user" and "approved" in str(message.get("content", "")).lower():
            approval_user_seen = True
        for tool_call in message.get("tool_calls", []):
            function = tool_call.get("function", {})
            name = function.get("name")
            arguments = function.get("arguments", {})
            if name not in tools_by_name:
                raise ValueError(f"{record.get('id')}: unknown tool {name}")
            if name not in offered:
                raise ValueError(f"{record.get('id')}: called tool {name} is not in its tools list")
            if not isinstance(arguments, dict):
                raise ValueError(f"{record.get('id')}: arguments for {name} must be an object")
            validate_args(tools_by_name[name], arguments)
            xml = render_tool_call_xml(name, arguments)
            parsed_name, parsed_args = parse_tool_call_xml(xml)
            if parsed_name != name or parsed_args != arguments:
                raise ValueError(f"{record.get('id')}: tool XML round-trip failed for {name}")
            calls_seen.append((name, arguments))
        if message.get("role") == "tool" and not message.get("content"):
            raise ValueError(f"{record.get('id')}: tool response cannot be empty")
    if record.get("policy") == "approval" and not (approval_text_seen and approval_user_seen):
        raise ValueError(f"{record.get('id')}: approval policy needs an approval request and user approval")
    if record.get("mode") == "auto":
        for name, _ in calls_seen:
            if tools_by_name[name].get("risky") and record.get("policy") != "approval":
                raise ValueError(f"{record.get('id')}: risky Auto Mode call {name} lacks approval policy")


def write_jsonl(path: Path, records: list[dict[str, Any]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as handle:
        for record in records:
            handle.write(json.dumps(record, ensure_ascii=False, separators=(",", ":")) + "\n")


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle if line.strip()]


def split_records(records: list[dict[str, Any]], seed: int, eval_fraction: float, tools_by_name: dict[str, dict[str, Any]]) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    target = max(1, int(round(len(records) * eval_fraction)))
    # Keep one first-seen direct example per tool and representative policy cases in evaluation.
    eval_indices: set[int] = set()
    for index, record in enumerate(records):
        names = {
            call_item.get("function", {}).get("name")
            for message in record["messages"]
            for call_item in message.get("tool_calls", [])
        }
        if names and len(eval_indices) < len(tools_by_name):
            eval_indices.add(index)
            if len(eval_indices) == len(tools_by_name):
                break
    for policy in ("approval", "sequence", "capability"):
        for index, record in enumerate(records):
            if record.get("policy") == policy:
                eval_indices.add(index)
                if policy != "approval":
                    break
    rng = random.Random(seed)
    remaining = [index for index in range(len(records)) if index not in eval_indices]
    rng.shuffle(remaining)
    for index in remaining:
        if len(eval_indices) >= max(target, len(tools_by_name) + 6):
            break
        eval_indices.add(index)
    train = [record for index, record in enumerate(records) if index not in eval_indices]
    evaluation = [record for index, record in enumerate(records) if index in eval_indices]
    rng.shuffle(train)
    rng.shuffle(evaluation)
    return train, evaluation


def prepare(args: argparse.Namespace) -> None:
    tools = load_tools()
    by_name = tool_map(tools)
    records = build_records(tools)
    for record in records:
        validate_record(record, by_name)
    config = load_config()
    train, evaluation = split_records(records, config["seed"], config["eval_split"], by_name)
    if len(train) < 1 or len(evaluation) < len(by_name):
        raise RuntimeError("dataset split did not produce a usable training/evaluation set")
    if args.validate_only:
        print(f"validated {len(records)} records ({len(train)} train, {len(evaluation)} eval)")
        print(f"validated {len(by_name)} Conduit tools")
        return
    write_jsonl(TRAIN_PATH, train)
    write_jsonl(EVAL_PATH, evaluation)
    manifest = {
        "total_records": len(records),
        "train_records": len(train),
        "eval_records": len(evaluation),
        "tools": sorted(by_name),
        "seed": config["seed"],
        "eval_fraction": config["eval_split"],
    }
    (DATA_DIR / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))


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


def load_dataset_for_training(path: Path):
    from datasets import Dataset
    records = read_jsonl(path)
    if not records:
        raise RuntimeError(f"dataset is empty: {path}")
    return Dataset.from_list(records)


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
        completion_only_loss=False,
        assistant_only_loss=False,
        remove_unused_columns=False,
        dataset_num_proc=1,
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
    prepare(argparse.Namespace(validate_only=False))
    model, tokenizer, torch = load_training_stack(config, smoke=smoke, with_vision_data=args.with_vision_data)
    train_dataset = load_dataset_for_training(TRAIN_PATH)
    eval_dataset = load_dataset_for_training(EVAL_PATH)
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


def template_check(args: argparse.Namespace) -> None:
    """Render every record through Ornith's real chat template without loading weights."""
    config = load_config()
    if not TRAIN_PATH.exists() or not EVAL_PATH.exists():
        prepare(argparse.Namespace(validate_only=False))
    try:
        from transformers import AutoProcessor
    except ImportError as exc:
        raise RuntimeError("template-check requires the Unsloth/Transformers Python environment") from exc
    processor = AutoProcessor.from_pretrained(
        config["model_id"],
        cache_dir=str(ROOT / config["model_cache_dir"]),
    )
    tokenizer = getattr(processor, "tokenizer", processor)
    count = 0
    max_tokens = 0
    longest = None
    for path in (TRAIN_PATH, EVAL_PATH):
        for record in read_jsonl(path):
            rendered = processor.apply_chat_template(
                record["messages"],
                tools=record["tools"],
                tokenize=False,
                add_generation_prompt=False,
                enable_thinking=False,
            )
            if any(item["function"]["name"] not in rendered for item in record.get("tools", [])) and "<tool_call>" in rendered:
                raise ValueError(f"{record['id']}: rendered tool call lost its function name")
            tokenized = tokenizer.apply_chat_template(
                record["messages"],
                tools=record["tools"],
                tokenize=True,
                add_generation_prompt=False,
                enable_thinking=False,
            )
            if hasattr(tokenized, "input_ids"):
                token_count = len(tokenized.input_ids)
            elif isinstance(tokenized, dict) and "input_ids" in tokenized:
                token_count = len(tokenized["input_ids"])
            else:
                token_count = len(tokenized)
            if token_count > max_tokens:
                max_tokens, longest = token_count, record["id"]
            count += 1
    if max_tokens > int(config["max_seq_length"]):
        raise ValueError(f"{longest}: {max_tokens} tokens exceeds max_seq_length={config['max_seq_length']}")
    print(f"rendered {count} records with Ornith's chat template; max_tokens={max_tokens} ({longest})")


def evaluate(args: argparse.Namespace) -> None:
    tools = load_tools()
    by_name = tool_map(tools)
    if not EVAL_PATH.exists():
        prepare(argparse.Namespace(validate_only=False))
    records = read_jsonl(EVAL_PATH)
    report: dict[str, Any] = {"records": len(records), "tool_calls": 0, "approval_cases": 0, "failures": []}
    seen_tools: set[str] = set()
    for record in records:
        try:
            validate_record(record, by_name)
            for message in record["messages"]:
                for item in message.get("tool_calls", []):
                    name = item["function"]["name"]
                    seen_tools.add(name)
                    report["tool_calls"] += 1
            if record.get("policy") == "approval":
                report["approval_cases"] += 1
        except Exception as exc:
            report["failures"].append({"id": record.get("id"), "error": str(exc)})
    report["tools_covered"] = sorted(seen_tools)
    report["tool_coverage"] = round(len(seen_tools) / len(by_name), 3)
    report["valid"] = not report["failures"]
    output = ROOT / load_config()["output_dir"]
    output.mkdir(parents=True, exist_ok=True)
    (output / "evaluation.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2))
    if report["failures"]:
        raise SystemExit(1)


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
    sub.add_parser("evaluate", help="validate and score the held-out policy/tool set")
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
