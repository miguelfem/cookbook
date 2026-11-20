"""Fictional profile lookups with an injected wait. No network access."""
from concurrent.futures import ThreadPoolExecutor
import argparse
import json
import re
import sys
import time


def profile(email):
    match = re.fullmatch(r"person(0[1-9]|1[0-6])@example\.test", email)
    if match is None:
        raise ValueError("Only the 16 fixture addresses are accepted")
    number = int(match[1])
    time.sleep(0.22 + 0.02 * (number % 4))
    return {
        "name": f"Example Person {number:02d}",
        "organization": ("Cedar Lab", "Fable Studio", "Harbor Works")[number % 3],
        "role": ("Designer", "Engineer", "Researcher", "Writer")[number % 4],
    }


def main():
    if len(sys.argv) == 1:
        envelope = json.load(sys.stdin)
        print(json.dumps({"protocol_version": "0.1", "output": profile(envelope["input"]["email"])}))
        return
    parser = argparse.ArgumentParser()
    parser.add_argument("--workers", type=int, choices=(1, 4), default=4)
    parser.add_argument("emails", nargs="+")
    args = parser.parse_args()
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        profiles = [
            {"key": email, "output": result}
            for email, result in zip(args.emails, pool.map(profile, args.emails), strict=True)
        ]
    print(json.dumps({"profiles": profiles}, separators=(",", ":")))


if __name__ == "__main__":
    main()
