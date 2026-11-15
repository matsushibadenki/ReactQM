"""Local ASE optimization using the existing validated PySCF adapter."""
import hashlib
import json
import math
import os
import tempfile
from pathlib import Path

from .calculation import atomic_json, evaluate, validate_request


def optimize(request: dict, directory: Path, fmax: float, steps: int, resume: bool, *, on_step=None) -> dict:
    import ase
    import numpy as np
    import pyscf
    from ase import Atoms, units
    from ase.calculators.calculator import Calculator, all_changes
    from ase.optimize import BFGS

    validate_request(request)
    if not math.isfinite(fmax) or not 0.001 <= fmax <= 1.0 or not 1 <= steps <= 100:
        raise ValueError("Invalid force threshold or step budget")
    document = request["document"]
    if len(document["atoms"]) < 2:
        raise ValueError("Optimization requires at least two atoms")
    identity = {
        "schema_version": 1,
        "request_sha256": hashlib.sha256(
            json.dumps(request, sort_keys=True, allow_nan=False).encode()
        ).hexdigest(),
        "job_id": request["job_id"],
        "input_hash": request["input_hash"],
        "method": request["method"],
        "basis": request["basis"],
        "charge": document["charge"],
        "spin": document["spin"],
        "max_cycle": request["max_cycle"],
        "conv_tol": request["conv_tol"],
        "settings": request.get("settings"),
        "optimizer": "ASE BFGS",
        "ase_version": ase.__version__,
        "pyscf_version": pyscf.__version__,
        "fmax_ev_per_angstrom": fmax,
        "maxstep_angstrom": 0.1,
    }
    from pyscf import dft
    if request["method"] in ("RKS", "UKS"):
        identity["libxc_version"] = dft.libxc.__version__
    if on_step is not None:
        with (directory / "input.json").open("rb") as stream:
            data = stream.read(65537)
        if len(data) > 65536 or json.loads(data) != request:
            raise ValueError("Stored calculation input does not match the optimization request")
        identity["input_file_sha256"] = hashlib.sha256(data).hexdigest()
    checkpoint = directory / "optimization-checkpoint.json"
    result_path = directory / "optimization-result.json"
    if result_path.exists():
        raise ValueError("Optimization already completed")
    positions = [atom["position_angstrom"] for atom in document["atoms"]]
    completed = 0
    if resume:
        with checkpoint.open("rb") as stream:
            data = stream.read(32769)
        if len(data) > 32768:
            raise ValueError("Oversized checkpoint")
        saved = json.loads(data)
        if saved["identity"] != identity:
            raise ValueError("Checkpoint does not match the input, settings or engine versions")
        positions = saved["positions_angstrom"]
        completed = saved["steps"]
        if type(completed) is not int or not 0 <= completed <= 10000:
            raise ValueError("Invalid checkpoint step")
    elif on_step is None:
        # A fresh directory prevents accidentally overwriting another run.
        directory.mkdir(parents=True, exist_ok=False)

    def at_geometry(coordinates):
        if len(coordinates) != len(document["atoms"]):
            raise ValueError("Invalid geometry size")
        return {**request, "document": {**document, "atoms": [
            {**atom, "position_angstrom": position}
            for atom, position in zip(document["atoms"], coordinates, strict=True)
        ]}}

    validate_request(at_geometry(positions))

    class PySCFCalculator(Calculator):
        implemented_properties = ["energy", "forces"]

        def calculate(self, atoms=None, properties=("energy",), system_changes=all_changes):
            super().calculate(atoms, properties, system_changes)
            energy, gradient, _, _ = evaluate(at_geometry(self.atoms.positions.tolist()), lambda _: None)
            self.results = {
                "energy": energy * units.Hartree,
                "forces": -gradient * units.Hartree / units.Bohr,
            }

    atoms = Atoms(numbers=[atom["atomic_number"] for atom in document["atoms"]], positions=positions)
    atoms.calc = PySCFCalculator()
    with BFGS(atoms, maxstep=identity["maxstep_angstrom"], logfile=None) as optimizer:
        def record():
            state = {
                "identity": identity,
                "steps": completed + optimizer.nsteps,
                "positions_angstrom": atoms.positions.tolist(),
                "energy_hartree": atoms.get_potential_energy() / units.Hartree,
                "max_force_ev_per_angstrom": float(np.linalg.norm(atoms.get_forces(), axis=1).max()),
            }
            atomic_json(checkpoint, state)
            if on_step is not None:
                on_step(state)

        optimizer.attach(record)
        optimizer.run(fmax=fmax, steps=min(steps, 10000 - completed))
        record()
    result = json.loads(checkpoint.read_bytes())
    result["converged"] = result["max_force_ev_per_angstrom"] < fmax
    if result["converged"] and on_step is None:
        from ase.io import write

        with tempfile.NamedTemporaryFile(mode="w", dir=directory, delete=False) as stream:
            temporary = Path(stream.name)
            try:
                write(stream, atoms, format="xyz")
                stream.flush()
                os.fsync(stream.fileno())
                os.replace(temporary, directory / "optimized.xyz")
            finally:
                temporary.unlink(missing_ok=True)
        atomic_json(result_path, result)
    return result


def run_cli(input_path, directory, fmax, steps, resume):
    import sys

    try:
        import fcntl
        with input_path.open("rb") as stream:
            data = stream.read(65537)
        if len(data) > 65536:
            raise ValueError("Oversized calculation input")
        request = json.loads(data)
        # The OS releases this lock on interruption, including SIGKILL.
        directory = directory.resolve()
        directory.parent.mkdir(parents=True, exist_ok=True)
        with directory.with_name(directory.name + ".lock").open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = optimize(request, directory, fmax, steps, resume)
        print(json.dumps(result, allow_nan=False))
        return 0 if result["converged"] else 2
    except Exception as error:
        print(f"Optimization failed: {error}", file=sys.stderr)
        return 1
