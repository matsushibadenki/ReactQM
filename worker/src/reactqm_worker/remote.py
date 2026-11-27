"""Opt-in loopback HTTP queue for existing ReactQM single-point requests."""
import hashlib
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, HTTPServer
import hmac
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import tempfile
from threading import Event, Thread
import time
import uuid

from .calculation import validate_request

from .remote_contract import ARTIFACTS, MAX_ARTIFACT, MAX_REQUEST, store_lease

ROUTE = re.compile(r"/v1/jobs/([0-9a-f-]{36})(?:/(input\.json|result\.json|result\.h5))?")


class Queue:
    def __init__(self, root):
        self.root = root.resolve()
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.db = self.root / "remote.sqlite3"
        with self.connect() as db:
            db.execute("""CREATE TABLE IF NOT EXISTS jobs (
                sequence INTEGER PRIMARY KEY, id TEXT UNIQUE NOT NULL, digest TEXT NOT NULL,
                request BLOB NOT NULL, status TEXT NOT NULL, attempts INTEGER NOT NULL DEFAULT 0,
                artifact_dir TEXT, manifest TEXT
            )""")

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.db, timeout=5)
        try:
            with db:
                yield db
        finally:
            db.close()

    def recover(self):
        with self.connect() as db:
            db.execute("UPDATE jobs SET status='cancelled' WHERE status='cancelling'")
            db.execute("UPDATE jobs SET status=CASE WHEN attempts < 2 THEN 'queued' ELSE 'failed' END WHERE status='running'")

    def submit(self, data):
        request = json.loads(data)
        identifier = request["job_id"]
        if str(uuid.UUID(identifier)) != identifier:
            raise ValueError("invalid_job_id")
        allowed = {"schema_version", "job_id", "document_revision", "input_hash", "document",
                   "method", "basis", "max_cycle", "conv_tol", "settings", "remote_port"}
        if set(request) - allowed or request["method"] not in ("RHF", "UHF", "RKS", "UKS"):
            raise ValueError("unsupported_remote_job")
        if not 1 <= len(request["document"]["atoms"]) <= 16:
            raise ValueError("unsupported_remote_job")
        validate_request(request)
        canonical = json.dumps(request, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()
        if len(canonical) + 1 > MAX_REQUEST:
            raise ValueError("request_too_large")
        digest = hashlib.sha256(canonical).hexdigest()
        with self.connect() as db:
            db.execute("BEGIN IMMEDIATE")
            previous = db.execute("SELECT digest FROM jobs WHERE id=?", (identifier,)).fetchone()
            if previous:
                if previous[0] != digest:
                    raise FileExistsError("job_id_conflict")
            else:
                if db.execute("SELECT count(*) FROM jobs WHERE status IN ('queued','running','cancelling')").fetchone()[0] >= 100:
                    raise OverflowError("queue_full")
                db.execute("INSERT INTO jobs(id,digest,request,status) VALUES (?,?,?,'queued')",
                           (identifier, digest, canonical))
        return identifier, previous is None

    def claim(self):
        with self.connect() as db:
            db.execute("BEGIN IMMEDIATE")
            row = db.execute("SELECT id,request FROM jobs WHERE status='queued' ORDER BY sequence LIMIT 1").fetchone()
            if row:
                db.execute("UPDATE jobs SET status='running', attempts=attempts+1 WHERE id=?", (row[0],))
        return row

    def status(self, identifier):
        with self.connect() as db:
            row = db.execute("SELECT status,attempts,manifest FROM jobs WHERE id=?", (identifier,)).fetchone()
        if row is None:
            return None
        return {"schema_version": 1, "job_id": identifier, "status": row[0], "attempts": row[1],
                "artifacts": json.loads(row[2]) if row[0] == "succeeded" and row[2] else []}

    def cancel(self, identifier):
        with self.connect() as db:
            db.execute("""UPDATE jobs SET status=CASE status
                          WHEN 'queued' THEN 'cancelled' WHEN 'running' THEN 'cancelling' END
                          WHERE id=? AND status IN ('queued','running')""", (identifier,))
        return self.status(identifier)

    def work(self, stop):
        while not stop.is_set():
            row = self.claim()
            if row is None:
                stop.wait(0.5)
                continue
            identifier, payload = row
            try:
                self.execute(identifier, payload, stop)
            except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
                with self.connect() as db:
                    db.execute("""UPDATE jobs SET status=CASE
                               WHEN status='cancelling' THEN 'cancelled'
                               WHEN attempts < 2 THEN 'queued' ELSE 'failed' END
                               WHERE id=? AND status IN ('running','cancelling')""",
                               (identifier,))

    def execute(self, identifier, payload, stop):
        import shutil

        folder = Path(tempfile.mkdtemp(prefix=f"{identifier}-", dir=self.root))
        published = False
        try:
            (folder / "input.json").write_bytes(payload)
            env = {key: value for key, value in os.environ.items()
                   if key in ("PATH", "PYTHONPATH", "SYSTEMROOT", "TMPDIR", "TEMP")}
            env.update(OMP_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1", MKL_NUM_THREADS="1")
            deadline = time.monotonic() + 120
            with subprocess.Popen(
                [sys.executable, "-m", "reactqm_worker", "calculate", "--output-dir", str(folder)],
                stdin=subprocess.PIPE, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            ) as child:
                try:
                    data = payload + b"\n"
                    while True:
                        if stop.is_set() or self.status(identifier)["status"] != "running":
                            raise InterruptedError("remote_job_interrupted")
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("remote_job_timed_out")
                        try:
                            child.communicate(input=data, timeout=min(0.5, remaining))
                            break
                        except subprocess.TimeoutExpired:
                            data = None
                    if child.returncode:
                        raise subprocess.CalledProcessError(child.returncode, child.args)
                finally:
                    if child.poll() is None:
                        child.kill()
                        child.wait()
            metadata = []
            for name in ARTIFACTS:
                with (folder / name).open("rb") as stream:
                    size = os.fstat(stream.fileno()).st_size
                    if not 0 < size <= MAX_ARTIFACT:
                        raise ValueError("invalid_artifact_size")
                    digest = hashlib.file_digest(stream, "sha256").hexdigest()
                    os.fsync(stream.fileno())
                metadata.append({"name": name, "bytes": size, "sha256": digest})
            result = json.loads((folder / "result.json").read_bytes())
            source = json.loads(payload)
            if (result.get("job_id") != identifier or result.get("converged") is not True
                    or any(result.get(key) != source.get(key) for key in ("input_hash", "method", "basis", "settings"))):
                raise ValueError("invalid_result_identity")
            with self.connect() as db:
                cursor = db.execute(
                    """UPDATE jobs SET status='succeeded',artifact_dir=?,manifest=?
                       WHERE id=? AND status='running'""",
                    (folder.name, json.dumps(metadata), identifier),
                )
                if cursor.rowcount != 1:
                    raise InterruptedError("remote_job_interrupted")
            published = True
        finally:
            if not published:
                shutil.rmtree(folder, ignore_errors=True)

    def artifact(self, identifier, name):
        with self.connect() as db:
            row = db.execute("SELECT artifact_dir,manifest FROM jobs WHERE id=? AND status='succeeded'",
                             (identifier,)).fetchone()
        if row is None:
            return None
        expected = next(entry for entry in json.loads(row[1]) if entry["name"] == name)
        with (self.root / row[0] / name).open("rb") as stream:
            data = stream.read(MAX_ARTIFACT + 1)
        if len(data) != expected["bytes"] or hashlib.sha256(data).hexdigest() != expected["sha256"]:
            raise ValueError("artifact_checksum")
        return data, expected["sha256"]


def serve(root, port):
    token = os.environ.get("REACTQM_REMOTE_TOKEN", "")
    if not re.fullmatch(r"[!-~]{32,256}", token):
        raise ValueError("REACTQM_REMOTE_TOKEN must contain 32–256 printable ASCII characters without spaces")
    stop = Event()

    class Handler(BaseHTTPRequestHandler):
        def setup(self):
            super().setup()
            self.connection.settimeout(10)

        def send_data(self, status, data, media, digest=None):
            self.send_response(status)
            self.send_header("Content-Type", media)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            if digest:
                self.send_header("X-Content-SHA256", digest)
            self.end_headers()
            self.wfile.write(data)

        def reply(self, status, value):
            self.send_data(status, json.dumps(value).encode(), "application/json")

        def authorized(self):
            if self.headers.get("Host") != f"127.0.0.1:{self.server.server_port}" or self.headers.get("Origin") is not None:
                self.reply(403, {"error": "invalid_origin"})
                return False
            if not hmac.compare_digest(self.headers.get("Authorization", "").encode(), f"Bearer {token}".encode()):
                self.reply(401, {"error": "unauthorized"})
                return False
            return True

        def do_POST(self):
            if not self.authorized():
                return
            if self.path != "/v1/jobs":
                self.reply(404, {"error": "not_found"})
                return
            try:
                size = int(self.headers.get("Content-Length", "0"))
                if self.headers.get("Transfer-Encoding") or not 1 <= size <= MAX_REQUEST:
                    raise ValueError("invalid_request_length")
                if self.headers.get_content_type() != "application/json":
                    raise ValueError("invalid_content_type")
                identifier, created = queue.submit(self.rfile.read(size))
                self.reply(201 if created else 200, queue.status(identifier))
            except FileExistsError:
                self.reply(409, {"error": "job_id_conflict"})
            except OverflowError:
                self.reply(429, {"error": "queue_full"})
            except (KeyError, TypeError, ValueError):
                self.reply(400, {"error": "invalid_job"})
            except sqlite3.Error:
                self.reply(503, {"error": "queue_unavailable"})

        def do_DELETE(self):
            if not self.authorized():
                return
            match = ROUTE.fullmatch(self.path)
            if not match or match.group(2) is not None:
                self.reply(404, {"error": "not_found"})
                return
            try:
                state = queue.cancel(match.group(1))
                if state is None:
                    self.reply(404, {"error": "not_found"})
                else:
                    self.reply(200, state)
            except sqlite3.Error:
                self.reply(503, {"error": "queue_unavailable"})

        def do_GET(self):
            if not self.authorized():
                return
            match = ROUTE.fullmatch(self.path)
            if not match:
                self.reply(404, {"error": "not_found"})
                return
            identifier, name = match.groups()
            try:
                if name:
                    result = queue.artifact(identifier, name)
                    if result is not None:
                        data, digest = result
                        self.send_data(200, data, "application/octet-stream", digest)
                        return
                else:
                    result = queue.status(identifier)
                    if result is not None:
                        self.reply(200, result)
                        return
                self.reply(404, {"error": "not_found"})
            except (OSError, ValueError):
                self.reply(409, {"error": "artifact_invalid"})
            except sqlite3.Error:
                self.reply(503, {"error": "queue_unavailable"})

    root = root.resolve()
    root.mkdir(mode=0o700, parents=True, exist_ok=True)
    with store_lease(root):
        queue = Queue(root)
        queue.recover()
        with HTTPServer(("127.0.0.1", port), Handler) as server:
            worker = Thread(target=queue.work, args=(stop,))
            worker.start()
            print(f"ReactQM remote worker listening on 127.0.0.1:{server.server_port}", file=sys.stderr)
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
            finally:
                stop.set()
                worker.join()
