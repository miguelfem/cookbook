#!/usr/bin/env python3
"""Small fixed-attachment MCP client for disposable E02 setup and readback."""
import json
import select
import subprocess
import time
from pathlib import Path

class Client:
    def __init__(self, apr, state, attachment, stderr_path):
        apr = Path(apr)
        if not apr.is_file():
            raise FileNotFoundError(f"APR executable does not exist: {apr}")
        self.stderr_file = Path(stderr_path).open("a")
        self.process = subprocess.Popen(
            [str(apr), "--state-dir", str(state), "mcp", "--attachment-file", str(attachment)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr_file,
            text=True, bufsize=1)
        self.sequence = 0
        self.request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                  "clientInfo": {"name": "e02-disposable-setup", "version": "1"}})
        self.process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
        self.process.stdin.flush()

    def request(self, method, params):
        self.sequence += 1
        number = self.sequence
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": number,
                                              "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            ready, _, _ = select.select([self.process.stdout], [], [], 1)
            if ready:
                response = json.loads(self.process.stdout.readline())
                if response.get("id") == number:
                    return response
        raise TimeoutError(f"MCP {method} timed out")

    def tool(self, name, arguments):
        response = self.request("tools/call", {"name": name, "arguments": arguments})
        if "error" in response or response.get("result", {}).get("isError"):
            raise RuntimeError(f"MCP {name}: {response}")
        return response["result"]["structuredContent"]

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=10)
        self.stderr_file.close()
