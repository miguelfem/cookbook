"""Single-owner trigger relay. Only the paired CookBook worker starts recipes."""

from contextlib import closing
import hashlib
import hmac
from http import HTTPStatus
import json
import os
import re
import secrets
import sqlite3
import time

MAX_BODY = 65536
LEASE_SECONDS = 60
SLUG = re.compile(r"[a-z0-9][a-z0-9-]{0,63}")
JOB_ID = re.compile(r"[0-9a-f]{32}")
RUN_ID = re.compile(r"run_[0-9a-f]{32}")

os.umask(0o077)
DATABASE = "/data/relay.sqlite3"
RECIPES = set(os.environ["APR_RELAY_RECIPES"].split(","))
if not all(SLUG.fullmatch(name) for name in RECIPES):
    raise RuntimeError("APR_RELAY_RECIPES must contain comma-separated recipe names")


def bearer(name):
    token = os.environ[name]
    if not re.fullmatch(r"[A-Za-z0-9_-]{32,256}", token):
        raise RuntimeError(f"{name} needs a randomly generated URL-safe token")
    return f"Bearer {token}".encode("ascii")


SUBMIT_AUTH = bearer("APR_RELAY_SUBMIT_TOKEN")
WORKER_AUTH = bearer("APR_RELAY_WORKER_TOKEN")
if hmac.compare_digest(SUBMIT_AUTH, WORKER_AUTH):
    raise RuntimeError("Use separate submit and worker tokens")

with closing(sqlite3.connect(DATABASE, timeout=5)) as connection, connection:
    connection.execute(
        """CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY,
            request_key TEXT NOT NULL UNIQUE,
            fingerprint TEXT NOT NULL,
            recipe TEXT NOT NULL,
            input TEXT,
            lease TEXT,
            lease_until INTEGER,
            run_id TEXT
        )"""
    )


class RequestError(Exception):
    def __init__(self, status, message):
        self.status = status
        self.message = message


def json_body(env):
    if env.get("CONTENT_TYPE", "").split(";", 1)[0].strip() != "application/json":
        raise RequestError(415, "Expected application/json")
    length = int(env.get("CONTENT_LENGTH") or 0)
    if length > MAX_BODY:
        raise RequestError(413, "Input exceeds 64 KiB")
    if length < 1:
        raise RequestError(400, "Expected a JSON object")
    body = json.loads(env["wsgi.input"].read(length))
    if not isinstance(body, dict):
        raise RequestError(400, "Expected a JSON object")
    return body


def submit(db, body, now):
    key, recipe, value = (body.get(name) for name in ("request_key", "recipe", "input"))
    if (
        set(body) != {"request_key", "recipe", "input"}
        or not isinstance(key, str)
        or not SLUG.fullmatch(key)
        or not isinstance(recipe, str)
        or recipe not in RECIPES
        or not isinstance(value, dict)
    ):
        raise RequestError(400, "Expected a request key, allowed recipe and input object")
    payload = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False)
    fingerprint = hashlib.sha256(f"{recipe}\n{payload}".encode()).hexdigest()
    old = db.execute("SELECT * FROM jobs WHERE request_key = ?", (key,)).fetchone()
    if old:
        if old["fingerprint"] != fingerprint:
            raise RequestError(409, "Request key already belongs to different input")
        return 202, status(old, now)
    job_id = secrets.token_hex(16)
    db.execute(
        "INSERT INTO jobs (id,request_key,fingerprint,recipe,input) VALUES (?,?,?,?,?)",
        (job_id, key, fingerprint, recipe, payload),
    )
    return 202, {"job_id": job_id, "status": "queued"}


def status(row, now):
    if row["run_id"]:
        return {"job_id": row["id"], "status": "submitted", "run_id": row["run_id"]}
    claimed = row["lease_until"] is not None and row["lease_until"] > now
    return {"job_id": row["id"], "status": "claimed" if claimed else "queued"}


def claim(db, body, now):
    if body:
        raise RequestError(400, "Expected an empty claim object")
    job = db.execute(
        """SELECT id, recipe, input FROM jobs
           WHERE run_id IS NULL AND (lease_until IS NULL OR lease_until <= ?)
           ORDER BY rowid LIMIT 1""",
        (now,),
    ).fetchone()
    if job is None:
        return 200, {"job": None}
    lease = secrets.token_hex(16)
    db.execute(
        "UPDATE jobs SET lease = ?, lease_until = ? WHERE id = ?",
        (lease, now + LEASE_SECONDS, job["id"]),
    )
    return 200, {"job": {
        "id": job["id"], "recipe": job["recipe"],
        "input": json.loads(job["input"]), "lease": lease,
    }}


def acknowledge(db, body, now):
    job_id, lease, run_id = (body.get(name) for name in ("job_id", "lease", "run_id"))
    fields = ((JOB_ID, job_id), (JOB_ID, lease), (RUN_ID, run_id))
    if set(body) != {"job_id", "lease", "run_id"} or not all(
        isinstance(value, str) and pattern.fullmatch(value) for pattern, value in fields
    ):
        raise RequestError(400, "Invalid receipt")
    updated = db.execute(
        """UPDATE jobs SET run_id = ?, input = NULL, lease = NULL, lease_until = NULL
           WHERE id = ? AND lease = ? AND lease_until > ? AND run_id IS NULL""",
        (run_id, job_id, lease, now),
    )
    if not updated.rowcount:
        raise RequestError(409, "Claim expired or was already acknowledged")
    return 200, {"job_id": job_id, "status": "submitted", "run_id": run_id}


def route(env):
    method, path = env["REQUEST_METHOD"], env["PATH_INFO"]
    if (method, path) == ("GET", "/healthz"):
        return 200, {"status": "ok"}
    action = {
        ("POST", "/jobs"): submit,
        ("POST", "/claims"): claim,
        ("POST", "/receipts"): acknowledge,
    }.get((method, path))
    prefix = "/jobs/"
    job_id = path.removeprefix(prefix)
    lookup = method == "GET" and path.startswith(prefix) and JOB_ID.fullmatch(job_id)
    if action is None and not lookup:
        raise RequestError(404, "Unknown route")
    expected = WORKER_AUTH if action in (claim, acknowledge) else SUBMIT_AUTH
    actual = env.get("HTTP_AUTHORIZATION", "").encode("utf-8")
    if not hmac.compare_digest(actual, expected):
        raise RequestError(401, "Invalid credentials")
    body = json_body(env) if action else None
    with closing(sqlite3.connect(DATABASE, timeout=5)) as db, db:
        db.row_factory = sqlite3.Row
        if action:
            db.execute("BEGIN IMMEDIATE")
            return action(db, body, int(time.time()))
        row = db.execute("SELECT * FROM jobs WHERE id = ?", (job_id,)).fetchone()
        if row is None:
            raise RequestError(404, "Unknown job")
        return 200, status(row, int(time.time()))


def app(env, start_response):
    try:
        code, value = route(env)
    except RequestError as error:
        code, value = error.status, {"error": error.message}
    except (ValueError, OverflowError, RecursionError):
        code, value = 400, {"error": "Invalid JSON request"}
    except sqlite3.OperationalError:
        code, value = 503, {"error": "Queue temporarily unavailable"}
    body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    start_response(f"{code} {HTTPStatus(code).phrase}", [
        ("Content-Type", "application/json"),
        ("Content-Length", str(len(body))),
        ("Cache-Control", "no-store"),
    ])
    return [body]
