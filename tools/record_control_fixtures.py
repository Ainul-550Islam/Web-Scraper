#!/usr/bin/env python3
"""Record the control-plane fixtures the console's tests render against.

Starts a local mock origin and a real `harvest serve`, drives the four
control-plane operations the console exposes, and writes the JSON responses
into `web/src/test/fixtures/`. Nothing here is invented: every byte under
`web/src/test/fixtures/control_*.json` came out of the admin server.

Usage:  python3 tools/record_control_fixtures.py
"""
from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "web", "src", "test", "fixtures")
BIN = os.path.join(ROOT, "target", "debug", "harvest")
TOKEN = "record-control-fixture-token-0123456789"


class Origin(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802
        if self.path == "/robots.txt":
            body, ctype = b"User-agent: *\nAllow: /\n", "text/plain"
        elif self.path in ("/", "/index.html"):
            links = "".join(f'<a href="/p/{i}">p{i}</a>' for i in range(1, 4))
            body = f"<html><body><h1>Catalogue</h1>{links}</body></html>".encode()
            ctype = "text/html"
        else:
            ident = self.path.rsplit("/", 1)[-1]
            body = (
                f"<html><body><h1>Item {ident}</h1>"
                f"<p class=price>${ident}.50</p></body></html>"
            ).encode()
            ctype = "text/html"
        self.send_response(200)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args: object) -> None:
        return


def free_port() -> int:
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class Client:
    def __init__(self, addr: str) -> None:
        self.base = f"http://{addr}"

    def call(self, method: str, path: str, body: dict | None = None):
        data = json.dumps(body).encode() if body is not None else None
        req = urllib.request.Request(
            self.base + path,
            data=data,
            headers={
                "authorization": f"Bearer {TOKEN}",
                "content-type": "application/json",
            },
            method=method,
        )
        try:
            with urllib.request.urlopen(req, timeout=30) as resp:
                return resp.status, json.loads(resp.read().decode())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read().decode())


def wait_job(c: Client, job_id: str, timeout: float = 60.0) -> dict:
    end = time.time() + timeout
    while time.time() < end:
        st, snap = c.call("GET", f"/control/jobs/{job_id}")
        assert st == 200, snap
        if snap["state"] not in ("queued", "running"):
            return snap
        time.sleep(0.1)
    raise SystemExit(f"job {job_id} never finished")


def main() -> int:
    if not os.path.exists(BIN):
        print("build first: cargo build --bin harvest", file=sys.stderr)
        return 2

    origin = HTTPServer(("127.0.0.1", free_port()), Origin)
    threading.Thread(target=origin.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{origin.server_address[1]}"

    tmp = tempfile.mkdtemp(prefix="harvest-fixture-")
    admin = f"127.0.0.1:{free_port()}"
    cfg = {
        "name": "console-fixture-run",
        "respect_robots": True,
        "concurrency": 2,
        "max_pages": 20,
        "request_timeout_secs": 5,
        "metrics_addr": admin,
        "admin_token": TOKEN,
        "schedule": {
            "every_secs": 3600,
            "runs": 0,
            "history_path": os.path.join(tmp, "out/runs.jsonl"),
        },
        "output": {"kind": "jsonl", "path": os.path.join(tmp, "out/records.jsonl")},
        "targets": [
            {
                "id": "catalogue",
                "start_url": f"{base}/index.html",
                "follow_links": True,
                "max_depth": 1,
                "max_pages": 10,
                "extract": {"title": {"css": "h1"}, "price": {"css": "p.price"}},
            }
        ],
    }
    cfg_path = os.path.join(tmp, "cfg.json")
    with open(cfg_path, "w") as fh:
        json.dump(cfg, fh)

    proc = subprocess.Popen(
        [BIN, "serve", "--config", cfg_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        # wait for the admin listener
        end = time.time() + 20
        while time.time() < end:
            try:
                socket.create_connection(("127.0.0.1", int(admin.split(":")[1])), 0.5).close()
                break
            except OSError:
                time.sleep(0.1)
        # wait for the startup run to land a history line
        end = time.time() + 60
        while time.time() < end:
            hp = os.path.join(tmp, "out/runs.jsonl")
            if os.path.exists(hp) and os.path.getsize(hp) > 0:
                break
            time.sleep(0.1)

        c = Client(admin)

        # 1. an export with artifacts (this is job-1)
        st, export = c.call(
            "POST",
            "/control/export",
            {"op": "export", "formats": ["csv", "jsonl", "xlsx", "zip"], "verify": True},
        )
        assert st == 202, export
        job_1 = wait_job(c, export["id"])

        # 2. a crawl (job-2) — registers the run output as an artifact
        st, crawl = c.call("POST", "/control/crawl", {"op": "crawl", "targets": ["catalogue"]})
        assert st == 202, crawl
        wait_job(c, crawl["id"])

        # 3. an AI round on the disabled provider (job-3): a failed job with a
        #    reason, so the console has a real failure to render
        st, ai = c.call("POST", "/control/ai", {"op": "ai", "task": "classify", "provider": "disabled"})
        assert st == 202, ai
        wait_job(c, ai["id"])

        st, jobs = c.call("GET", "/control/jobs?limit=20")
        assert st == 200, jobs

        payload = next(
            (a for a in job_1["artifacts"] if a["name"] == "records.csv"),
            job_1["artifacts"][0],
        )
        art_id = payload["id"]

        st, artifact = c.call("GET", f"/control/artifacts/{art_id}")
        assert st == 200, artifact
        st, verified = c.call("GET", f"/artifacts/{art_id}/verify")
        assert st == 200, verified

        os.makedirs(OUT, exist_ok=True)
        written = [
            ("control_jobs.json", jobs),
            ("control_job.json", job_1),
            ("control_artifact.json", artifact),
            ("control_artifact_verify.json", verified),
        ]
        for name, data in written:
            path = os.path.join(OUT, name)
            with open(path, "w") as fh:
                json.dump(data, fh, indent=2, sort_keys=True)
                fh.write("\n")
            print(f"wrote {os.path.relpath(path, ROOT)}")
        ids = {
            "job": job_1["id"],
            "crawl_job": crawl["id"],
            "ai_job": ai["id"],
            "artifact": art_id,
        }
        print(json.dumps(ids, indent=2))
        return 0
    finally:
        proc.kill()
        proc.wait()
        origin.shutdown()
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
