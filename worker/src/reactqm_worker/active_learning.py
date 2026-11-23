"""Rank completed MACE-OFF23 predictions and run a bounded PySCF reference batch."""
import hashlib
import math
import time
from pathlib import Path

from .mace_potential import CHECKPOINT_SHA256, MEDIUM_SHA256, check_document
from .stoichiometry import read_json

SETTINGS = {"method": "RKS", "basis": "def2-svp", "xc": "PBE0", "grid_level": 3,
            "max_cycle": 50, "conv_tol": 1e-9}


def rank(candidates, budget):
    """Keep one copy of each exact geometry and rank without reference labels."""
    ordered = sorted(candidates, key=lambda item: (-item["score"], item["job_id"]))
    result = []
    seen = set()
    for item in ordered:
        if item["geometry_key"] not in seen:
            result.append(item)
            seen.add(item["geometry_key"])
            if len(result) == budget:
                break
    return result


def load_candidates(manifest_path):
    manifest, digest = read_json(manifest_path)
    directories = manifest["jobs"]
    budget = manifest["budget"]
    if (not isinstance(directories, list) or not 1 <= len(directories) <= 100
            or type(budget) is not int or not 1 <= budget <= min(20, len(directories))):
        raise ValueError("Use 1–100 jobs and a reference budget of 1–20")
    candidates = []
    excluded = []
    for directory in directories:
        path = manifest_path.parent / directory
        job, source_digest = read_json(path / "input.json")
        result, result_digest = read_json(path / "result.json")
        if (job.get("method") != "MACE-OFF23" or result.get("method") != "MACE-OFF23"
                or result.get("job_id") != job["job_id"] or result.get("input_hash") != job["input_hash"]
                or result.get("converged") is not True):
            raise ValueError(f"Invalid MACE result for {directory}")
        report = result["mace"]
        if report["checkpoint_sha256"] != CHECKPOINT_SHA256:
            raise ValueError(f"Unexpected primary checkpoint for {directory}")
        numbers, positions = check_document(job["document"])
        diagnostic = (report.get("diagnostics") or {}).get("disagreement")
        if any(z > 10 for z in numbers) or diagnostic is None:
            excluded.append({"job_id": job["job_id"],
                             "reason": "reference_element_scope" if any(z > 10 for z in numbers)
                             else "missing_medium_disagreement"})
            continue
        score = diagnostic["max_force_ev_per_angstrom"]
        forces = report["forces_ev_per_angstrom"]
        if (diagnostic["checkpoint_sha256"] != MEDIUM_SHA256 or not math.isfinite(score) or score < 0
                or len(forces) != len(numbers) or any(len(force) != 3 or not all(map(math.isfinite, force)) for force in forces)):
            raise ValueError(f"Invalid MACE force diagnostics for {directory}")
        candidates.append({"job_id": job["job_id"], "input_hash": job["input_hash"],
                           "source_sha256": source_digest, "result_sha256": result_digest,
                           "document": job["document"], "score": score, "forces": forces,
                           "geometry_key": tuple(zip(numbers, map(tuple, positions), strict=True))})
    return rank(candidates, budget), excluded, digest, budget


def run(manifest_path):
    from ase import units
    import numpy as np
    import pyscf

    from .calculation import evaluate

    targets, excluded, digest, budget = load_candidates(manifest_path)
    pyscf.lib.num_threads(1)
    observations = []
    for target in targets:
        request = {"schema_version": 1, "job_id": target["job_id"], "input_hash": target["input_hash"],
                   "document": target["document"], "settings": SETTINGS,
                   **{key: SETTINGS[key] for key in ("method", "basis", "max_cycle", "conv_tol")}}
        started = time.monotonic()
        summary = {key: target[key] for key in ("job_id", "input_hash", "source_sha256", "result_sha256")}
        summary["max_model_disagreement_ev_per_angstrom"] = target["score"]
        try:
            energy, gradient, _, _ = evaluate(request, lambda _: None)
        except ArithmeticError as error:
            if str(error) != "not_converged":
                raise
            summary["status"] = "not_converged"
        else:
            reference = -gradient * units.Hartree / units.Bohr
            differences = np.linalg.norm(reference - np.asarray(target["forces"]), axis=1)
            summary.update(status="succeeded", reference_energy_hartree=energy,
                           reference_forces_ev_per_angstrom=reference.tolist(),
                           max_force_difference_ev_per_angstrom=float(differences.max()))
        summary["elapsed_seconds"] = time.monotonic() - started
        observations.append(summary)
    return {"schema_version": 1, "manifest_sha256": digest, "budget": budget,
            "ranking": "max_small_medium_force_difference", "reference_settings": SETTINGS,
            "checkpoint_sha256": CHECKPOINT_SHA256, "medium_sha256": MEDIUM_SHA256,
            "pyscf_version": pyscf.__version__, "excluded": excluded, "selected": observations}
