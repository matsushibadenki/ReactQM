"""Select and test an OFF23 warning threshold on disjoint molecular groups."""
import argparse
from collections import Counter, defaultdict
import contextlib
from functools import lru_cache
import hashlib
import json
from pathlib import Path
import random
import sys

import numpy as np
from rdkit import Chem

from mace_heldout import eligible_frames
from reactqm_worker.mace_potential import diagnostics, evaluate_document

ERROR_CUTOFF = 0.50
BASELINE_THRESHOLD = 0.50
CANDIDATES = (0.10, 0.15, 0.20, 0.25, 0.30, 0.40, 0.50, 0.60, 0.75, 1.00)


@lru_cache(maxsize=16384)
def molecule_key(smiles):
    if not isinstance(smiles, str):
        return None
    molecule = Chem.MolFromSmiles(smiles)
    if molecule is None:
        return None
    for atom in molecule.GetAtoms():
        atom.SetAtomMapNum(0)
    # Group stereoisomers together so opposite stereochemistry cannot cross splits.
    return Chem.MolToSmiles(molecule, isomericSmiles=False)


def sample(path, per_stratum, seed):
    rng = random.Random(seed)
    samples = defaultdict(list)
    counts = Counter()
    excluded = Counter()
    for index, document, reference, info in eligible_frames(path, excluded):
        key = molecule_key(info.get("smiles"))
        if not key:
            excluded["missing_molecule_identity"] += 1
            continue
        category = info.get("config_type")
        if not isinstance(category, str) or not category:
            excluded["missing_category"] += 1
            continue
        digest = hashlib.sha256(f"{seed}:{key}".encode()).digest()
        split = "calibration" if digest[0] < 128 else "validation"
        atoms = document["atoms"]
        geometry = diagnostics([atom["atomic_number"] for atom in atoms],
                               [atom["position_angstrom"] for atom in atoms])
        tags = []
        if geometry["minimum_ratio"] < 0.80:
            tags.append("compressed")
        if geometry["connectivity_ratio"] > 1.20:
            tags.append("stretched")
        kind = "_and_".join(tags) or "regular"
        stratum = (split, category, kind)
        counts[stratum] += 1
        reservoir = samples[stratum]
        slot = len(reservoir) if len(reservoir) < per_stratum else rng.randrange(counts[stratum])
        if slot < per_stratum:
            item = (index, key, document, reference)
            if slot == len(reservoir):
                reservoir.append(item)
            else:
                reservoir[slot] = item
    return samples, counts, excluded


def metrics(rows, threshold):
    counts = dict(tp=0, fp=0, tn=0, fn=0)
    for row in rows:
        warning = row["geometry"] != "regular" or row["disagreement"] > threshold
        high = row["max_error"] > ERROR_CUTOFF
        counts["tp" if warning and high else "fp" if warning else "fn" if high else "tn"] += 1
    positive = counts["tp"] + counts["fn"]
    negative = counts["tn"] + counts["fp"]
    return {**counts, "sensitivity": counts["tp"] / positive if positive else None,
            "specificity": counts["tn"] / negative if negative else None}


def main(args):
    sampled, counts, excluded = sample(args.xyz, args.per_stratum, args.seed)
    rows = {"calibration": [], "validation": []}
    for (split, category, geometry), frames in sorted(sampled.items()):
        for index, key, document, reference in sorted(frames):
            with contextlib.redirect_stdout(sys.stderr):
                result = evaluate_document(document, args.small, None, args.medium)
            forces = np.asarray(result["forces_ev_per_angstrom"])
            delta = result["diagnostics"]["disagreement"]
            rows[split].append({"frame": index, "molecule": key, "category": category,
                                "geometry": geometry, "disagreement": delta["max_force_ev_per_angstrom"],
                                "max_error": float(np.linalg.norm(forces - reference, axis=1).max())})
    if not rows["calibration"] or not rows["validation"]:
        raise ValueError("Both molecular splits must contain eligible structures")
    calibration_molecules = {row["molecule"] for row in rows["calibration"]}
    if any(row["molecule"] in calibration_molecules for row in rows["validation"]):
        raise RuntimeError("Molecular identity crossed the split")
    trial = [(threshold, metrics(rows["calibration"], threshold)) for threshold in CANDIDATES]
    ranked = [(0.5 * (score["sensitivity"] + score["specificity"]), -abs(threshold - BASELINE_THRESHOLD), threshold)
              for threshold, score in trial if score["sensitivity"] is not None and score["specificity"] is not None]
    chosen = max(ranked)[2] if ranked else None
    validation = rows["validation"]
    groups = defaultdict(list)
    for row in validation:
        groups[row["category"], row["geometry"]].append(row)
    with args.xyz.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
    print(json.dumps({
        "dataset_doi": "10.17863/CAM.107498", "xyz_sha256": digest, "seed": args.seed,
        "per_stratum_limit": args.per_stratum, "excluded_frames": excluded, "unit": "eV/angstrom",
        "small_sha256": result["scope"]["checkpoint_sha256"], "medium_sha256": delta["checkpoint_sha256"],
        "mace_version": result["mace_version"], "torch_version": result["torch_version"],
        "population": [{"split": a, "category": b, "geometry": c, "count": n}
                       for (a, b, c), n in sorted(counts.items())],
        "maximum_error_cutoff": ERROR_CUTOFF, "candidate_calibration_metrics": trial,
        "selected_threshold": chosen, "current_threshold": BASELINE_THRESHOLD,
        "validation_current": metrics(validation, BASELINE_THRESHOLD),
        "validation_selected": metrics(validation, chosen) if chosen is not None else None,
        "validation_by_stratum": [
            {"category": category, "geometry": geometry,
             "current": metrics(group, BASELINE_THRESHOLD),
             "selected": metrics(group, chosen) if chosen is not None else None}
            for (category, geometry), group in sorted(groups.items())
        ],
        "frames": rows,
    }, indent=2, allow_nan=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("xyz", type=Path)
    parser.add_argument("small", type=Path)
    parser.add_argument("medium", type=Path)
    parser.add_argument("--per-stratum", type=int, default=8)
    parser.add_argument("--seed", type=int, default=5041)
    args = parser.parse_args()
    if not 2 <= args.per_stratum <= 100:
        parser.error("--per-stratum must be in 2–100")
    main(args)
