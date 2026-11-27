"""Opt-in stdio MCP access to bounded, isolated ReactQM calculations."""
import contextlib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from threading import Lock
from typing import Literal
import uuid

from ase.data import atomic_numbers
from mcp.server.fastmcp import FastMCP
from mcp.types import ToolAnnotations

calculation_gate = Lock()
server = FastMCP("ReactQM", instructions="Use supplied molecular geometries for local single points. Report units and method limits.")


@server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=False))
def research_capabilities() -> dict:
    """Describe the local calculation contract and scientific scope."""
    return {"methods": ["RHF", "PBE0"], "basis": ["sto-3g", "6-31g", "def2-svp"],
            "elements": "H–Ne", "max_atoms": 16, "charge": 0, "spin_2s": 0,
            "coordinate_unit": "angstrom", "energy_unit": "hartree",
            "storage": "temporary isolated Worker directory"}


def document_from_xyz(xyz):
    lines = xyz.strip().splitlines()
    if len(xyz.encode()) > 4096 or len(lines) < 3:
        raise ValueError("Use one XYZ geometry with 1–16 atoms")
    count = int(lines[0])
    if not 1 <= count <= 16 or len(lines) != count + 2:
        raise ValueError("XYZ atom count must match the geometry")
    atoms = []
    for line in lines[2:]:
        symbol, x, y, z = line.split()
        atoms.append({"atomic_number": atomic_numbers[symbol], "position_angstrom": list(map(float, (x, y, z)))})
    return {"schema_version": 1, "name": "MCP geometry", "coordinate_unit": "angstrom",
            "charge": 0, "spin": 0, "atoms": atoms}


@server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=False))
def single_point(
    xyz: str,
    method: Literal["RHF", "PBE0"] = "RHF",
    basis: Literal["sto-3g", "6-31g", "def2-svp"] = "sto-3g",
) -> dict:
    """Run a local PySCF single point on neutral closed-shell H–Ne XYZ (Å). Returns energy in hartree."""
    from .calculation import validate_request

    document = document_from_xyz(xyz)
    settings = {"method": "RHF" if method == "RHF" else "RKS", "basis": basis,
                "xc": None if method == "RHF" else "PBE0", "max_cycle": 50, "conv_tol": 1e-9,
                "grid_level": None if method == "RHF" else 3}
    raw_doc = json.dumps(document, sort_keys=True, allow_nan=False).encode()
    job = {"schema_version": 1, "job_id": str(uuid.uuid4()),
           "input_hash": hashlib.sha256(raw_doc).hexdigest(), "document": document, "settings": settings,
           **{key: settings[key] for key in ("method", "basis", "max_cycle", "conv_tol")}}
    validate_request(job)
    # The calculation receives no model API keys and has no access to the desktop catalog.
    env = {key: os.environ[key] for key in ("PATH", "PYTHONPATH", "SYSTEMROOT", "TMPDIR", "TEMP")
           if key in os.environ}
    env.update(OMP_NUM_THREADS="1", OPENBLAS_NUM_THREADS="1", MKL_NUM_THREADS="1")
    if not calculation_gate.acquire(blocking=False):
        raise ValueError("A local research calculation is already running")
    try:
        return _run_point_subprocess(job, env)
    finally:
        calculation_gate.release()


def _run_point_subprocess(job, env):
    with tempfile.TemporaryDirectory(prefix="reactqm-mcp-") as folder:
        # Research uses the same validated evaluator; no heavy field artifacts are generated.
        target = Path(folder)
        result = subprocess.run(
            [sys.executable, "-m", "reactqm_worker", "research-point"],
            input=json.dumps(job), text=True, cwd=target, env=env, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, timeout=60, check=True,
        )
    return json.loads(result.stdout)


def run_point():
    from importlib.metadata import version

    from .calculation import evaluate

    job = json.loads(sys.stdin.buffer.read(65536))
    with contextlib.redirect_stdout(sys.stderr):
        energy, _, _, _ = evaluate(job, lambda _: None)
    print(json.dumps({"energy_hartree": energy, "settings": job["settings"], "input_sha256": job["input_hash"],
                      "engine": "PySCF", "engine_version": version("pyscf"), "converged": True}, allow_nan=False))
    return 0


def _catalog_root():
    configured = os.environ.get("REACTQM_MCP_CATALOG")
    if not configured:
        raise ValueError("The MCP catalog is not configured")
    root = Path(configured)
    if not root.is_absolute():
        raise ValueError("REACTQM_MCP_CATALOG must be an absolute directory path")
    return root.resolve()


def _catalog_query(root, sql, args):
    import sqlite3
    with contextlib.closing(sqlite3.connect(
        (root / "jobs.sqlite3").as_uri() + "?mode=ro", uri=True, timeout=2,
    )) as connection:
        return connection.execute(sql, args).fetchall()


def _bounded_file(path, limit=65536):
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ValueError("Catalog artifact exceeds the supported size")
    return data


if os.environ.get("REACTQM_MCP_CATALOG"):
    @server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=False))
    def recent_results(limit: int = 10) -> list[dict]:
        """List up to 25 completed job IDs and molecular formulas from the opted-in local catalog."""
        if not 1 <= limit <= 25:
            raise ValueError("limit must be 1–25")
        rows = _catalog_query(_catalog_root(),
                              "SELECT id, formula, created_at FROM jobs WHERE status = 'succeeded' ORDER BY sequence DESC LIMIT ?",
                              (limit,))
        return [{"job_id": job, "formula": formula, "created_at": created} for job, formula, created in rows]

    @server.tool(annotations=ToolAnnotations(readOnlyHint=True, openWorldHint=False))
    def result_summary(job_id: str) -> dict:
        """Read receipt-verified calculation metadata for a completed ReactQM job. Energy is in hartree."""
        if str(uuid.UUID(job_id)) != job_id:
            raise ValueError("Invalid job ID")
        root = _catalog_root()
        rows = _catalog_query(root,
                              "SELECT formula, receipt_sha256 FROM jobs WHERE id = ? AND status = 'succeeded'",
                              (job_id,))
        if not rows:
            raise ValueError("Completed result not found")
        formula, digest = rows[0]
        directory = root / "jobs" / job_id
        if directory.resolve() != directory:
            raise ValueError("Invalid job directory")
        receipt_bytes = _bounded_file(directory / "completion.json")
        if hashlib.sha256(receipt_bytes).hexdigest() != digest:
            raise ValueError("Completion receipt mismatch")
        receipt = json.loads(receipt_bytes)
        result_bytes = _bounded_file(directory / "result.json")
        artifact = next((entry for entry in receipt["artifacts"] if entry["name"] == "result.json"), None)
        if (receipt["schema_version"] != 1 or receipt["job_id"] != job_id or artifact is None
                or artifact["bytes"] != len(result_bytes)
                or artifact["sha256"] != hashlib.sha256(result_bytes).hexdigest()):
            raise ValueError("Result checksum mismatch")
        result = json.loads(result_bytes)
        if result["job_id"] != job_id or result["input_hash"] != receipt["input_hash"]:
            raise ValueError("Result identity mismatch")
        keys = ("job_id", "input_hash", "method", "basis", "energy_hartree", "engine_version", "converged", "settings")
        summary = {"formula": formula, **{key: result[key] for key in keys if key in result}}
        for field in ("escalation", "xtb_followup"):
            if field in result:
                summary["workflow_outcome"] = result[field]["policy_v1"]["decision"]
        return summary
