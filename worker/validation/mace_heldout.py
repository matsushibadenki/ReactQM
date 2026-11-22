"""Audit OFF23's fixed warnings on the authors' held-out extended XYZ data."""
import argparse
from collections import Counter
import contextlib
import hashlib
import json
from pathlib import Path
import random
import sys

from ase.io import iread
import numpy as np
from scipy.stats import spearmanr

from reactqm_worker.mace_potential import check_document, evaluate_document


def eligible_frames(path, excluded):
    for index, atoms in enumerate(iread(path, index=":", format="extxyz")):
        if atoms.info.get("total_charge") != 0 or atoms.pbc.any():
            excluded["charge_or_periodicity"] += 1
            continue
        document = {
            "schema_version": 1, "coordinate_unit": "angstrom", "charge": 0, "spin": 0,
            "atoms": [{"atomic_number": int(z), "position_angstrom": xyz.tolist()}
                      for z, xyz in zip(atoms.numbers, atoms.positions, strict=True)],
        }
        try:
            check_document(document)
        except ValueError as error:
            excluded[str(error)] += 1
            continue
        # ASE places the published "forces" column in the single-point results.
        reference = atoms.calc.results.get("forces") if atoms.calc else None
        if reference is None or reference.shape != (len(atoms), 3) or not np.isfinite(reference).all():
            raise ValueError(f"Missing or invalid reference forces at frame {index}")
        yield index, document, reference, atoms.info


def sample_frames(path, size, seed):
    rng = random.Random(seed)
    sample = []
    excluded = Counter()
    eligible = 0
    for index, document, reference, info in eligible_frames(path, excluded):
        eligible += 1
        slot = len(sample) if len(sample) < size else rng.randrange(eligible)
        if slot < size:
            item = (index, document, reference, info.get("config_type"))
            if slot == len(sample):
                sample.append(item)
            else:
                sample[slot] = item
    return sorted(sample, key=lambda frame: frame[0]), eligible, excluded


def main(args):
    with args.xyz.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    frames, eligible, excluded = sample_frames(args.xyz, args.samples, args.seed)
    if len(frames) < 2:
        raise ValueError("At least two eligible structures are required")
    rows = []
    counts = dict(tp=0, fp=0, tn=0, fn=0)
    for index, document, reference, category in frames:
        with contextlib.redirect_stdout(sys.stderr):
            prediction = evaluate_document(document, args.small, None, args.medium)
        diagnostics = prediction["diagnostics"]
        disagreement = diagnostics["disagreement"]["max_force_ev_per_angstrom"]
        difference = np.linalg.norm(np.asarray(prediction["forces_ev_per_angstrom"]) - reference, axis=1)
        error = float(difference.max())
        warning = (diagnostics["minimum_ratio"] < 0.80 or diagnostics["connectivity_ratio"] > 1.20
                   or disagreement > 0.50)
        high = error > 0.50
        counts["tp" if warning and high else "fp" if warning else "fn" if high else "tn"] += 1
        rows.append({"frame": index, "category": category, "warning": bool(warning), "diagnostics": diagnostics,
                     "max_error_ev_per_angstrom": error,
                     "rms_error_ev_per_angstrom": float(np.sqrt(np.mean(difference**2)))})
    differences = [row["max_error_ev_per_angstrom"] for row in rows]
    disagreements = [row["diagnostics"]["disagreement"]["max_force_ev_per_angstrom"] for row in rows]
    rho = float(spearmanr(disagreements, differences).statistic)
    positives = counts["tp"] + counts["fn"]
    negatives = counts["tn"] + counts["fp"]
    print(json.dumps({
        "dataset_doi": "10.17863/CAM.107498", "xyz_sha256": digest, "seed": args.seed,
        "eligible_frames": eligible, "excluded_frames": excluded, "samples": len(rows),
        "small_sha256": prediction["scope"]["checkpoint_sha256"],
        "medium_sha256": diagnostics["disagreement"]["checkpoint_sha256"],
        "mace_version": prediction["mace_version"], "torch_version": prediction["torch_version"],
        "small_max_force_error_cutoff_ev_per_angstrom": 0.50,
        "spearman": rho if np.isfinite(rho) else None,
        "confusion": counts,
        "sensitivity": counts["tp"] / positives if positives else None,
        "specificity": counts["tn"] / negatives if negatives else None,
        "frames": rows,
    }, indent=2, allow_nan=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xyz", type=Path)
    parser.add_argument("small", type=Path)
    parser.add_argument("medium", type=Path)
    parser.add_argument("--samples", type=int, default=32)
    parser.add_argument("--seed", type=int, default=7127)
    args = parser.parse_args()
    if not 2 <= args.samples <= 1000:
        parser.error("--samples must be in 2–1000")
    main(args)
