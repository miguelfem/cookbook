"""One fixed, pinned CLI exposed as one modern stdio MCP tool. Stdlib only."""
import hashlib
import json
import os
import selectors
import signal
import subprocess
import sys
import tempfile
import time

MAX_MESSAGE = 65536


def invalid_constant(value):
    raise ValueError("non-JSON constant")


def digest(path):
    if not os.path.isabs(path):
        raise ValueError("dependency path is not absolute")
    with open(path, "rb") as stream:
        h = hashlib.sha256()
        total = 0
        while True:
            part = stream.read(65536)
            if not part:
                return h.hexdigest()
            total += len(part)
            if total > 512 * 1024 * 1024:
                raise ValueError("dependency is too large")
            h.update(part)


def run_cli(spec, arguments):
    if digest(spec["cli_executable"]) != spec["cli_sha256"]:
        raise ValueError("target CLI changed; prepare and review again")
    for dependency in spec["dependency_sha256"]:
        if digest(dependency["path"]) != dependency["sha256"]:
            raise ValueError("target dependency changed; prepare and review again")
    payload = json.dumps(arguments, separators=(",", ":"), ensure_ascii=False).encode("utf8") + b"\n"
    if len(payload) > spec["max_input_bytes"]:
        raise ValueError("tool input exceeds bound")
    env = {}
    credential_env = spec.get("credential_env")
    if credential_env:
        value = os.environ.get(credential_env)
        if value is None:
            raise ValueError("declared credential is unavailable")
        env[credential_env] = value
    with tempfile.TemporaryDirectory(prefix="cookbook-adapter-") as private_dir:
        process = subprocess.Popen(
            [spec["cli_executable"], *spec["cli_args"]],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            cwd=private_dir,
            env=env,
            close_fds=True,
            # Keep the target in CookBook's isolated adapter process group so
            # the outer Rust deadline kills both processes together.
            start_new_session=False,
        )
        try:
            return collect(process, payload, spec["max_output_bytes"], spec["timeout_ms"])
        except TimeoutError:
            # In the runtime the adapter leads a dedicated process group.
            # End that whole domain on a per-call timeout, including children.
            if os.getpgrp() == os.getpid():
                os.killpg(os.getpgrp(), signal.SIGKILL)
            raise
        finally:
            if process.poll() is None:
                try:
                    process.kill()
                except ProcessLookupError:
                    pass
            process.wait()


def collect(process, payload, maximum, timeout_ms):
    selector = selectors.DefaultSelector()
    selector.register(process.stdin, selectors.EVENT_WRITE)
    selector.register(process.stdout, selectors.EVENT_READ)
    sent = 0
    output = bytearray()
    deadline = time.monotonic() + timeout_ms / 1000
    while selector.get_map():
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("target CLI timed out")
        for key, _ in selector.select(remaining):
            if key.fileobj is process.stdin:
                try:
                    sent += os.write(process.stdin.fileno(), payload[sent:])
                except BrokenPipeError as error:
                    raise ValueError("target CLI closed stdin") from error
                if sent == len(payload):
                    selector.unregister(process.stdin)
                    process.stdin.close()
            else:
                part = os.read(process.stdout.fileno(), min(4096, maximum + 1 - len(output)))
                if not part:
                    selector.unregister(process.stdout)
                else:
                    output.extend(part)
                    if len(output) > maximum:
                        raise ValueError("target CLI output exceeds bound")
    try:
        status = process.wait(timeout=max(0.01, deadline - time.monotonic()))
    except subprocess.TimeoutExpired as error:
        raise TimeoutError("target CLI timed out") from error
    if status != 0:
        raise ValueError("target CLI failed")
    lines = output.splitlines()
    if len(lines) != 1:
        raise ValueError("target CLI must return one JSON line")
    result = json.loads(lines[0], parse_constant=invalid_constant)
    if not isinstance(result, dict):
        raise ValueError("target CLI must return a JSON object")
    return result


def reply(message, spec):
    method = message.get("method")
    params = message.get("params")
    meta = params.get("_meta") if isinstance(params, dict) else None
    if not isinstance(meta, dict) or meta.get("io.modelcontextprotocol/protocolVersion") != "2026-07-28":
        return {"error": {"code": -32602, "message": "Unsupported protocol metadata"}}
    if method == "server/discover":
        return {"result": {"resultType": "complete", "supportedVersions": ["2026-07-28"], "capabilities": {"tools": {}}}}
    if method == "tools/list":
        return {"result": {"resultType": "complete", "tools": [{"name": spec["name"], "inputSchema": spec["input_schema"], "outputSchema": spec["output_schema"]}]}}
    if method == "tools/call":
        if params.get("name") != spec["name"] or not isinstance(params.get("arguments"), dict):
            return {"error": {"code": -32602, "message": "Invalid tool or arguments"}}
        try:
            result = run_cli(spec, params["arguments"])
        except (OSError, ValueError, TimeoutError, json.JSONDecodeError):
            return {"error": {"code": -32000, "message": "Adapter call failed; inspect state before retry"}}
        return {"result": {"resultType": "complete", "structuredContent": result, "content": []}}
    return {"error": {"code": -32601, "message": "Method not found"}}


def main():
    # The bootstrap passes the exact bytes it verified; never reopen the path.
    spec = json.loads(PINNED_SPEC_BYTES, parse_constant=invalid_constant)
    for line in iter(lambda: sys.stdin.buffer.readline(MAX_MESSAGE + 1), b""):
        if len(line) > MAX_MESSAGE:
            break
        try:
            request = json.loads(line, parse_constant=invalid_constant)
            if not isinstance(request, dict) or request.get("jsonrpc") != "2.0" or "id" not in request:
                continue
            response = reply(request, spec)
            response.update(jsonrpc="2.0", id=request["id"])
            encoded = json.dumps(response, separators=(",", ":"), ensure_ascii=False)
            if len(encoded.encode("utf8")) > MAX_MESSAGE:
                response = {"jsonrpc": "2.0", "id": request["id"],
                            "error": {"code": -32000, "message": "Adapter result exceeds MCP response bound"}}
                encoded = json.dumps(response, separators=(",", ":"))
            sys.stdout.write(encoded + "\n")
            sys.stdout.flush()
        except (ValueError, TypeError, KeyError, json.JSONDecodeError):
            break


if __name__ == "__main__":
    main()
