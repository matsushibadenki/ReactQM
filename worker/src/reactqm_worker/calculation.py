"""Version 1 JSON-lines protocol for bounded RHF/STO-3G single-point jobs."""
import contextlib
import json
import math
import os
from pathlib import Path
import sys
import time
import uuid


def validate_request(request: dict) -> None:
    if request.get("schema_version") != 1 or request.get("method") != "RHF" or request.get("basis") != "sto-3g":
        raise ValueError("Unsupported job schema or method")
    uuid.UUID(request["job_id"])
    fingerprint = request["input_hash"]
    if len(fingerprint) != 64 or any(c not in "0123456789abcdef" for c in fingerprint):
        raise ValueError("Invalid input hash")
    if type(request["max_cycle"]) is not int or not 1 <= request["max_cycle"] <= 50:
        raise ValueError("Invalid cycle limit")
    if not isinstance(request["conv_tol"], (int, float)) or not 1e-12 <= request["conv_tol"] <= 1e-6:
        raise ValueError("Invalid convergence threshold")
    document = request["document"]
    if document["schema_version"] != 1 or document["coordinate_unit"] != "angstrom":
        raise ValueError("Unsupported document schema or unit")
    if type(document["charge"]) is not int or abs(document["charge"]) > 64 or type(document["spin"]) is not int or document["spin"] != 0:
        raise ValueError("RHF requires spin 2S = 0")
    atoms = document["atoms"]
    if not 1 <= len(atoms) <= 64:
        raise ValueError("Atom limit exceeded")
    for index, atom in enumerate(atoms):
        number, coordinates = atom["atomic_number"], atom["position_angstrom"]
        if type(number) is not int or not 1 <= number <= 10:
            raise ValueError("This engine milestone supports H through Ne")
        if len(coordinates) != 3 or any(type(v) not in (int, float) or not math.isfinite(v) or abs(v) > 10000 for v in coordinates):
            raise ValueError("Invalid coordinates")
        for other in atoms[:index]:
            if sum((a-b)**2 for a, b in zip(coordinates, other["position_angstrom"])) < 0.01:
                raise ValueError("Overlapping atoms")
    electrons = sum(atom["atomic_number"] for atom in atoms) - document["charge"]
    if electrons <= 0 or electrons % 2:
        raise ValueError("Inconsistent electron count")


def atomic_json(path: Path, value: dict) -> None:
    temporary = path.with_name(f".{uuid.uuid4()}.tmp")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(value, stream, allow_nan=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def calculate(request: dict, output_dir: Path, emit) -> dict:
    validate_request(request)
    import h5py
    import numpy as np
    import pyscf
    from pyscf import gto, scf

    started = time.monotonic()
    document = request["document"]
    output_dir.mkdir(parents=True, exist_ok=True)
    molecule = gto.M(atom=[(atom["atomic_number"], atom["position_angstrom"]) for atom in document["atoms"]],
                     unit="Angstrom", basis=request["basis"], charge=document["charge"], spin=0,
                     verbose=0, max_memory=512)
    solver = scf.RHF(molecule)
    solver.max_cycle = request["max_cycle"]
    solver.conv_tol = request["conv_tol"]
    solver.chkfile = str(output_dir / "scf.chk")
    solver.callback = lambda env: emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": int(env["cycle"]) + 1})
    energy = float(solver.kernel())
    if not solver.converged:
        raise ArithmeticError("not_converged")
    if not math.isfinite(energy):
        raise ValueError("Non-finite energy")
    gradients = solver.nuc_grad_method().kernel()
    if gradients.shape != (len(document["atoms"]), 3) or not np.isfinite(gradients).all():
        raise ValueError("Invalid gradient")
    temporary = output_dir / "result.h5.tmp"
    with h5py.File(temporary, "w") as artifact:
        artifact.attrs["schema_version"] = 1
        artifact.attrs["job_id"] = request["job_id"]
        artifact.attrs["input_hash"] = request["input_hash"]
        artifact.attrs["method"] = request["method"]
        artifact.attrs["basis"] = request["basis"]
        artifact.attrs["pyscf_version"] = pyscf.__version__
        artifact.create_dataset("energy", data=energy).attrs["unit"] = "hartree"
        artifact.create_dataset("gradient", data=gradients).attrs["unit"] = "hartree/bohr"
        artifact.create_dataset("coordinates", data=molecule.atom_coords(unit="Angstrom")).attrs["unit"] = "angstrom"
    with temporary.open("rb") as stream:
        os.fsync(stream.fileno())
    os.replace(temporary, output_dir / "result.h5")
    result = {"job_id": request["job_id"], "input_hash": request["input_hash"], "method": request["method"],
              "basis": request["basis"], "energy_hartree": energy, "converged": True,
              "engine_version": pyscf.__version__, "elapsed_seconds": time.monotonic() - started}
    atomic_json(output_dir / "result.json", result)
    return result


def serve(output_dir: Path) -> int:
    protocol_stdout = sys.stdout
    def emit(event):
        print(json.dumps(event, allow_nan=False), file=protocol_stdout, flush=True)
    request = {}
    try:
        line = sys.stdin.buffer.readline(32769)
        if len(line) > 32768:
            raise ValueError("Request too large")
        request = json.loads(line)
        # Engine/library chatter must not corrupt the control stream.
        with contextlib.redirect_stdout(sys.stderr):
            result = calculate(request, output_dir, emit)
        emit({"kind": "result", "schema_version": 1, "calculation": result})
        return 0
    except Exception as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        emit({"kind": "error", "schema_version": 1, "job_id": request.get("job_id", "") if isinstance(request, dict) else "",
              "code": "not_converged" if isinstance(error, ArithmeticError) and str(error) == "not_converged" else "calculation_failed"})
        return 1
