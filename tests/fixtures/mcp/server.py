"""Synthetic tools-only server: no network, package install, or real provider."""
import json
import os
import sys
import time
from pathlib import Path

mode, log = sys.argv[1:3]
for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    # Record method names only, not credential-bearing request bodies.
    with open(log, "a", encoding="utf8") as output:
        output.write(str(method) + "\n")
    if "id" not in message:
        continue
    modern = message.get("params", {}).get("_meta", {}).get("io.modelcontextprotocol/protocolVersion") == "2026-07-28"
    result = {"resultType": "complete"} if modern else {}
    if method == "initialize":
        result.update(protocolVersion=message["params"]["protocolVersion"], capabilities={"tools": {}}, serverInfo={"name": "synthetic", "version": "1"})
    elif method == "tools/list":
        schema = {"type": "object", "properties": {"value": {"type": "string" if mode == "drift" else "integer"}}, "required": ["value"], "additionalProperties": False}
        result["tools"] = [{"name": "normalize", "inputSchema": schema, "outputSchema": {"type": "object", "required": ["value"], "properties": {"value": {"type": "integer"}}}}]
        if mode in ("draft7", "draft7_bad_output"):
            for key in ("inputSchema", "outputSchema"):
                result["tools"][0][key]["$schema"] = "http://json-schema.org/draft-07/schema#"
        if mode == "app": result["tools"][0]["_meta"]={"ui":{"resourceUri":"ui://fixture/dashboard"}}
    elif method == "tools/call":
        if mode in ("mrtr", "sampling", "mixed") and "inputResponses" not in message["params"]:
            request = {"method":"elicitation/create", "params":{"mode":"form","message":"Choose a factor; never enter credentials", "requestedSchema":{"type":"object","properties":{"factor":{"type":"integer"}},"required":["factor"],"additionalProperties":False}}}
            if mode == "sampling":
                request = {"method":"sampling/createMessage", "params":{"messages":[{"role":"user","content":{"type":"text","text":"Summarize synthetic data"}}],"maxTokens":100}}
            result={"resultType":"input_required","inputRequests":{"review":request},"requestState":"opaque-state-private-sentinel"}
            if mode == "mixed":
                result["inputRequests"]["model"]={"method":"sampling/createMessage", "params":{"messages":[{"role":"user","content":{"type":"text","text":"Summarize synthetic data"}}],"maxTokens":100}}
            print(json.dumps({"jsonrpc":"2.0","id":message["id"],"result":result}),flush=True)
            continue
        if mode in ("mrtr", "sampling", "mixed"):
            assert message["params"]["requestState"] == "opaque-state-private-sentinel"
            assert "review" in message["params"]["inputResponses"]

        if mode == "timeout":
            time.sleep(30)
        if mode == "flood":
            print("x" * 100_000, flush=True)
            continue
        if mode == "interaction":
            print(json.dumps({"jsonrpc": "2.0", "id": 999, "method": "sampling/createMessage", "params": {}}), flush=True)
            continue
        if mode == "env":
            assert "HOME" not in os.environ and "PATH" not in os.environ
            assert os.environ.get("SERVICE_API_KEY") == "fixture-mcp-key"
        if mode == "child":
            import subprocess
            subprocess.Popen([sys.executable, "-I", "-c", "import time,pathlib,sys; time.sleep(.3); pathlib.Path(sys.argv[1]).write_text('escaped')", log + ".child"])
        result.update(content=[{"type": "text", "text": "fixture result"}], structuredContent={"value": message["params"]["arguments"]["value"]}, _meta={"usage": {"requests": 1}})
        if mode == "draft7_bad_output":
            result["structuredContent"]["value"] = "invalid integer"
        if mode == "error":
            result["isError"] = True
        if mode == "echo":
            result["content"][0]["text"] = os.environ["SERVICE_API_KEY"]
        if mode == "input_required":
            result["resultType"] = "input_required"
    elif method == "subscriptions/listen":
        meta={"io.modelcontextprotocol/subscriptionId":message["id"]}
        print(json.dumps({"jsonrpc":"2.0","method":"notifications/subscriptions/acknowledged","params":{"notifications":message["params"]["notifications"],"_meta":meta}}),flush=True)
        print(json.dumps({"jsonrpc":"2.0","method":"notifications/tools/list_changed","params":{"_meta":meta}}),flush=True)
        result.update(_meta=meta)
    elif method == "resources/list":
        result["resources"] = [{"uri":"ui://fixture/dashboard","name":"dashboard","mimeType":"text/html;profile=mcp-app"}] if mode=="app" else [{"uri":"fixture://record","name":"record","mimeType":"text/plain"}]
    elif method == "resources/read":
        if mode=="app":
            assert message["params"]["uri"]=="ui://fixture/dashboard"
            result["contents"]=[{"uri":"ui://fixture/dashboard","mimeType":"text/html;profile=mcp-app","text":"<h1>Synthetic App</h1>"}]
        else:
            assert message["params"]["uri"] == "fixture://record"
            result["contents"] = [{"uri":"fixture://record","text":"Synthetic resource"}]
    elif method == "prompts/list":
        result["prompts"] = [{"name":"summarize","arguments":[{"name":"topic","required":True}]}]
    elif method == "prompts/get":
        result["messages"] = [{"role":"user","content":{"type":"text","text":"Synthetic prompt"}}]
    else:
        raise RuntimeError("Unexpected method")
    print(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}), flush=True)
