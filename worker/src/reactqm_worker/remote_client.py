"""Submit, resume polling, and verify artifacts over the loopback Worker contract."""
import hashlib
from http.client import HTTPConnection, HTTPException
import json
import os
import shutil
import tempfile
import time
import uuid

from .remote_contract import ARTIFACTS, MAX_ARTIFACT, MAX_REQUEST


class RemoteJobMissing(ValueError):
    pass


def request(port, method, path, body=None, limit=MAX_REQUEST):
    token = os.environ.get("REACTQM_REMOTE_TOKEN")
    if not token:
        raise ValueError("REACTQM_REMOTE_TOKEN is required")
    connection = HTTPConnection("127.0.0.1", port, timeout=15)
    try:
        connection.request(method, path, body=body, headers={
            "Authorization": f"Bearer {token}", "Content-Type": "application/json",
        })
        response = connection.getresponse()
        if method == "DELETE" and response.status == 404:
            raise RemoteJobMissing("Remote Worker has no record of this job ID")
        if response.status not in (200, 201):
            raise ValueError(f"Remote Worker returned HTTP {response.status}")
        data = response.read(limit + 1)
        if len(data) > limit:
            raise ValueError("Remote response exceeds the supported size")
        return data
    finally:
        connection.close()


def cancel(job_id, port):
    if str(uuid.UUID(job_id)) != job_id:
        raise ValueError("Invalid job ID")
    state = json.loads(request(port, "DELETE", f"/v1/jobs/{job_id}"))
    if state["schema_version"] != 1 or state["job_id"] != job_id:
        raise ValueError("Remote job identity mismatch")
    return state


def run(input_path, output_dir, port, on_phase=None):
    if output_dir.exists():
        raise ValueError("Use a fresh output directory")
    with input_path.open("rb") as stream:
        payload = stream.read(MAX_REQUEST + 1)
    if len(payload) > MAX_REQUEST:
        raise ValueError("Input exceeds the supported size")
    original = json.loads(payload)
    identifier = original["job_id"]
    if str(uuid.UUID(identifier)) != identifier:
        raise ValueError("Invalid job ID")
    route = f"/v1/jobs/{identifier}"

    last_phase = None

    def publish(phase):
        nonlocal last_phase
        if on_phase is not None and phase != last_phase:
            on_phase(phase)
        last_phase = phase

    deadline = time.monotonic() + 300
    submitted = False
    while time.monotonic() < deadline:
        try:
            if submitted:
                state = json.loads(request(port, "GET", route))
            else:
                state = json.loads(request(port, "POST", "/v1/jobs", payload))
                submitted = True
        except (OSError, HTTPException):
            publish(4)  # Reconnecting.
            time.sleep(1)
            continue
        if state["schema_version"] != 1 or state["job_id"] != identifier:
            raise ValueError("Remote job identity mismatch")
        if state["status"] == "succeeded":
            publish(3)  # Retrieving artifacts.
            break
        if state["status"] == "archived":
            raise ValueError("Remote artifacts have been archived; use the archive store to retrieve them")
        if state["status"] == "cancelled":
            raise ValueError("Remote job was cancelled")
        if state["status"] not in ("queued", "running", "cancelling"):
            raise ValueError("Remote job failed")
        publish({"queued": 1, "running": 2, "cancelling": 5}[state["status"]])
        time.sleep(1)
    else:
        raise TimeoutError("Job still pending; rerun with the same input to retrieve it")

    manifest = {item["name"]: item for item in state["artifacts"]}
    if set(manifest) != set(ARTIFACTS) or len(state["artifacts"]) != len(ARTIFACTS):
        raise ValueError("Invalid remote artifact manifest")
    output_dir.parent.mkdir(parents=True, exist_ok=True)
    from pathlib import Path
    temporary = Path(tempfile.mkdtemp(prefix=".remote-", dir=output_dir.parent))
    reserved = False
    try:
        for name in ARTIFACTS:
            expected = manifest[name]
            size = expected["bytes"]
            limit = MAX_ARTIFACT if name == "result.h5" else MAX_REQUEST
            if type(size) is not int or not 1 <= size <= limit:
                raise ValueError("Invalid remote artifact size")
            data = request(port, "GET", f"{route}/{name}", limit=size)
            if len(data) != size or hashlib.sha256(data).hexdigest() != expected["sha256"]:
                raise ValueError("Remote artifact checksum mismatch")
            (temporary / name).write_bytes(data)
        if json.loads((temporary / "input.json").read_bytes()) != original:
            raise ValueError("Remote input does not match the submitted request")
        result = json.loads((temporary / "result.json").read_bytes())
        if result.get("job_id") != identifier or any(result.get(key) != original.get(key)
                for key in ("input_hash", "method", "basis", "settings")):
            raise ValueError("Remote result identity mismatch")
        (temporary / "remote-manifest.json").write_text(json.dumps(state, indent=2))
        output_dir.mkdir()
        reserved = True
        temporary.replace(output_dir)
        reserved = False
    finally:
        if reserved:
            output_dir.rmdir()
        shutil.rmtree(temporary, ignore_errors=True)
    return state
