"""Optional CPU energy/force inference with a pinned MACE-OFF23 checkpoint."""
import hashlib
import json
import math
from pathlib import Path
import tempfile

MODEL = "MACE-OFF23-small"
CHECKPOINT_SHA256 = "165cce4cfec5a34b9c64d4ebf95de15d71106bb584b7291c8470f0749977c46f"
ELEMENTS = {1: "H", 6: "C", 7: "N", 8: "O", 9: "F", 15: "P", 16: "S", 17: "Cl", 35: "Br", 53: "I"}
MAX_ATOMS = 64
MEDIUM_SHA256 = "4842c52ad210d6e1f84d6cf1ffa70fae25a7e0d755ed55cf223f43913f587db7"


def diagnostics(numbers, positions, disagreement=None):
    from ase.data import covalent_radii

    ratios = sorted((math.dist(positions[i], positions[j])
                     / (covalent_radii[numbers[i]] + covalent_radii[numbers[j]]), i, j)
                    for i in range(len(numbers)) for j in range(i))
    groups = list(range(len(numbers)))
    components = len(groups)
    for ratio, i, j in ratios:
        a, b = groups[i], groups[j]
        if a != b:
            groups = [a if group == b else group for group in groups]
            components -= 1
            if components == 1:
                break
    shortest, first, second = ratios[0]
    return {
        "schema_version": 1,
        "minimum_ratio": shortest, "closest_pair": [first, second],
        "connectivity_ratio": ratio, "connectivity_pair": [i, j],
        "disagreement": disagreement,
    }



def scope():
    return {
        "model": MODEL, "checkpoint_sha256": CHECKPOINT_SHA256,
        "elements": list(ELEMENTS.values()), "charge": 0, "spin_2s": 0,
        "atom_range": [2, MAX_ATOMS], "environment": "isolated nonperiodic molecule",
        "training_domain": "neutral closed-shell organic chemistry (SPICE v1)",
        "geometry_checks": {"min_pair_covalent_ratio": 0.60, "connectivity_covalent_ratio": 1.30},
        "validity": "Scope checks do not estimate uncertainty or establish training-set coverage.",
    }


def check_document(document):
    from ase.data import covalent_radii

    if document.get("schema_version") != 1 or document.get("coordinate_unit") != "angstrom":
        raise ValueError("mace_document_schema")
    if type(document.get("charge")) is not int or document["charge"] != 0:
        raise ValueError("mace_requires_neutral")
    if type(document.get("spin")) is not int or document["spin"] != 0:
        raise ValueError("mace_requires_singlet")
    if any(document.get(key) for key in ("cell", "pbc", "periodic")):
        raise ValueError("mace_requires_nonperiodic")
    atoms = document.get("atoms")
    if not isinstance(atoms, list) or not 2 <= len(atoms) <= MAX_ATOMS:
        raise ValueError("mace_atom_count")
    numbers = [atom["atomic_number"] for atom in atoms]
    if any(type(z) is not int or z not in ELEMENTS for z in numbers):
        raise ValueError("mace_unsupported_element")
    if sum(numbers) % 2:
        raise ValueError("mace_odd_electron_count")
    positions = [atom["position_angstrom"] for atom in atoms]
    if any(not isinstance(point, list) or len(point) != 3 or any(
        type(value) not in (int, float) or not math.isfinite(value) or abs(value) > 10000
        for value in point
    ) for point in positions):
        raise ValueError("mace_invalid_coordinates")

    neighbors = [set() for _ in atoms]
    for i, first in enumerate(positions):
        for j in range(i):
            ratio = math.dist(first, positions[j]) / (covalent_radii[numbers[i]] + covalent_radii[numbers[j]])
            if ratio < 0.60:
                raise ValueError("mace_geometry_overlap")
            if ratio <= 1.30:
                neighbors[i].add(j)
                neighbors[j].add(i)
    visited = {0}
    pending = [0]
    while pending:
        adjacent = neighbors[pending.pop()] - visited
        visited.update(adjacent)
        pending.extend(adjacent)
    if len(visited) != len(atoms):
        raise ValueError("mace_geometry_disconnected")
    return numbers, positions


def evaluate(input_path, model_path):
    with input_path.open("rb") as stream:
        source = stream.read(65537)
    if len(source) > 65536:
        raise ValueError("mace_input_too_large")
    return evaluate_document(json.loads(source), model_path, hashlib.sha256(source).hexdigest())


def evaluate_document(document, model_path, input_hash, medium_path=None):
    numbers, positions = check_document(document)

    import ase
    from ase import Atoms
    from importlib.metadata import version
    import numpy as np
    import torch
    from mace.calculators import MACECalculator

    torch.set_num_threads(1)
    def predict(path, expected_hash):
        # MACE checkpoints deserialize Python objects. Load only pinned official bytes.
        with path.open("rb") as stream:
            checkpoint = stream.read(32 * 1024 * 1024 + 1)
        if hashlib.sha256(checkpoint).hexdigest() != expected_hash:
            raise ValueError("mace_checkpoint_hash_mismatch")
        with tempfile.TemporaryDirectory(prefix="reactqm-mace-") as directory:
            verified = Path(directory) / "checkpoint.model"
            verified.write_bytes(checkpoint)
            calculator = MACECalculator(model_paths=str(verified), device="cpu", default_dtype="float64")
            if set(map(int, calculator.z_table.zs)) != set(ELEMENTS):
                raise ValueError("mace_checkpoint_element_mismatch")
            atoms = Atoms(numbers=numbers, positions=positions, pbc=False)
            atoms.calc = calculator
            energy = float(atoms.get_potential_energy())
            forces = atoms.get_forces()
        if not math.isfinite(energy) or forces.shape != (len(atoms), 3) or not np.isfinite(forces).all():
            raise ArithmeticError("mace_invalid_prediction")
        return energy, forces

    energy, forces = predict(model_path, CHECKPOINT_SHA256)
    disagreement = None
    if medium_path is not None:
        _, reference_forces = predict(medium_path, MEDIUM_SHA256)
        differences = np.linalg.norm(forces - reference_forces, axis=1)
        atom = int(np.argmax(differences))
        disagreement = {
            "checkpoint_sha256": MEDIUM_SHA256,
            "max_force_ev_per_angstrom": float(differences[atom]),
            "rms_force_ev_per_angstrom": float(np.sqrt(np.mean(differences**2))),
            "atom": atom,
        }
    return {
        "schema_version": 1, "scope": scope(), "input_sha256": input_hash,
        "mace_version": version("mace-torch"), "torch_version": str(torch.__version__),
        "ase_version": ase.__version__, "device": "cpu", "dtype": "float64",
        "energy_ev": energy, "forces_ev_per_angstrom": forces.tolist(),
        "diagnostics": diagnostics(numbers, positions, disagreement),
    }


def calculate_job(request, output_dir, emit):
    import os
    import time

    import h5py
    from .calculation import atomic_json

    started = time.monotonic()
    model = os.environ.get("REACTQM_MACE_MODEL")
    if not model or not Path(model).is_file():
        raise ArithmeticError("mace_checkpoint_missing")
    emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": 1})
    medium = os.environ.get("REACTQM_MACE_MEDIUM_MODEL")
    if medium and not Path(medium).is_file():
        raise ArithmeticError("mace_checkpoint_missing")
    try:
        prediction = evaluate_document(request["document"], Path(model), request["input_hash"],
                                       Path(medium) if medium else None)
    except ImportError as error:
        raise ArithmeticError("mace_runtime_missing") from error
    except ValueError as error:
        if str(error) == "mace_checkpoint_hash_mismatch":
            raise ArithmeticError(str(error)) from error
        raise
    output_dir.mkdir(parents=True, exist_ok=True)
    temporary = output_dir / "result.h5.tmp"
    with h5py.File(temporary, "w") as artifact:
        artifact.attrs["schema_version"] = 1
        artifact.attrs["job_id"] = request["job_id"]
        for key in ("input_sha256", "mace_version", "torch_version", "ase_version", "device", "dtype"):
            artifact.attrs[key] = prediction[key]
        artifact.attrs["diagnostics_json"] = json.dumps(prediction["diagnostics"], allow_nan=False)
        artifact.attrs["scope_json"] = json.dumps(prediction["scope"])
        artifact.create_dataset("energy", data=prediction["energy_ev"]).attrs["unit"] = "eV"
        artifact.create_dataset("forces", data=prediction["forces_ev_per_angstrom"]).attrs["unit"] = "eV/angstrom"
        artifact.create_dataset("coordinates", data=[
            atom["position_angstrom"] for atom in request["document"]["atoms"]
        ]).attrs["unit"] = "angstrom"
    with temporary.open("rb") as stream:
        os.fsync(stream.fileno())
    temporary.replace(output_dir / "result.h5")
    report = {key: prediction[key] for key in (
        "energy_ev", "forces_ev_per_angstrom", "torch_version", "ase_version", "diagnostics"
    )}
    report["checkpoint_sha256"] = CHECKPOINT_SHA256
    result = {
        "job_id": request["job_id"], "input_hash": request["input_hash"], "method": "MACE-OFF23",
        "basis": "small", "energy_hartree": prediction["energy_ev"] / 27.211386245988,
        "engine_version": prediction["mace_version"], "elapsed_seconds": time.monotonic() - started,
        "converged": True, "mace": report,
    }
    atomic_json(output_dir / "result.json", result)
    return result
