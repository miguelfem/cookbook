"""Scoped CookBook MCP client for synthetic provider evaluations.

Callers must supply the binary, state directory, and private attachment path.
This helper never prints or copies attachment contents.
"""

from __future__ import annotations

import json
import os
import select
import subprocess
import time
from pathlib import Path

class AprMcp:
    def __init__(self, *, apr: Path, state: Path, attachment: Path) -> None:
        if not apr.is_file() or not state.is_dir() or not attachment.is_file():
            raise ValueError("APR binary, state directory, and attachment file must exist")
        self.process = subprocess.Popen(
            [str(apr), "--state-dir", str(state), "mcp", "--attachment-file", str(attachment)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
            env=os.environ.copy(),
        )
        self.next_id = 1
        self.request(
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {},
             "clientInfo": {"name": "cookbook-synthetic-evals", "version": "0.1"}},
        )
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}) + "\n")
        self.process.stdin.flush()

    def request(self, method: str, params: dict, timeout: float = 45.0) -> dict:
        request_id = self.next_id
        self.next_id += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.process.stdout], [], [], remaining)[0]:
                raise TimeoutError(f"CookBook MCP {method} response uncertain")
            response = json.loads(self.process.stdout.readline())
            if response.get("id") == request_id:
                return response

    def call(self, name: str, arguments: dict, timeout: float = 45.0) -> dict:
        response = self.request("tools/call", {"name": name, "arguments": arguments}, timeout)
        if "error" in response:
            raise RuntimeError(f"MCP protocol error: {response['error'].get('code')}")
        return response["result"]

    def close(self) -> None:
        self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()

    def __enter__(self) -> "AprMcp":
        return self

    def __exit__(self, *_: object) -> None:
        self.close()
