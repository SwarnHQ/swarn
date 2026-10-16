#!/usr/bin/env python3
"""Stands in for `claude -p` stream-json in tests. Same message shapes as the recorded
claude-2.1.300.jsonl transcript: asks permission to write once, then echoes follow-ups."""
import json
import sys

def out(msg):
    print(json.dumps(msg), flush=True)

def turn_end(text):
    out({"type": "stream_event", "event": {"type": "content_block_delta", "index": 0,
         "delta": {"type": "text_delta", "text": text}}})
    out({"type": "result", "subtype": "success", "is_error": False, "stop_reason": "end_turn",
         "total_cost_usd": 0.0, "usage": {"input_tokens": 1, "output_tokens": 1}})

lines = iter(sys.stdin)
first = json.loads(next(lines))
out({"type": "system", "subtype": "init", "session_id": "fake-session", "model": "fake",
     "claude_code_version": "0.0.0-fake", "permissionMode": "manual"})
out({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "tool-1", "name": "Write",
     "input": {"file_path": "notes.txt", "content": first["message"]["content"]}}]}})
out({"type": "control_request", "request_id": "req-1", "request": {"subtype": "can_use_tool",
     "tool_name": "Write", "display_name": "Write", "description": "notes.txt",
     "input": {"file_path": "notes.txt"}, "permission_suggestions": []}})

for line in lines:
    msg = json.loads(line)
    if msg["type"] == "control_response":
        allowed = msg["response"]["response"]["behavior"] == "allow"
        out({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "tool-1",
             "is_error": not allowed, "content": "written" if allowed else "denied"}]}})
        turn_end("allowed" if allowed else "denied")
    elif msg["type"] == "user":
        turn_end("echo: " + msg["message"]["content"])
