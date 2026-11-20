"""A deliberately dependency-free trusted worker example."""
import json
import sys

request = json.load(sys.stdin)
text = " ".join(request["input"]["text"].split())
if request["config"]["uppercase"]:
    text = text.upper()
json.dump({"protocol_version": "0.1", "output": {
    "normalized": text,
    "word_count": len(text.split()),
}}, sys.stdout)
