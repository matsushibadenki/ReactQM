"""Measure OFF23 force errors against independent PySCF reference calculations."""
import argparse
import contextlib
import json
from pathlib import Path
import sys
import uuid

from ase import units
from ase.data import covalent_radii
import numpy as np
import pyscf
from rdkit import Chem, rdBase
from rdkit.Chem import AllChem
from scipy.stats import spearmanr

from reactqm_worker.calculation import evaluate
from reactqm_worker.mace_potential import evaluate_document


def main(small, medium):
    pyscf.lib.num_threads(1)
    rows = []
    settings = {"method": "RKS", "basis": "def2-svp", "xc": "PBE0", "grid_level": 3,
                "max_cycle": 50, "conv_tol": 1e-9}
    # Distinct molecules and seed from the original warning challenge.
    for name, smiles in (("ethanol", "CCO"), ("dimethyl ether", "COC"),
                         ("methylamine", "CN"), ("fluoromethane", "CF")):
        molecule = Chem.AddHs(Chem.MolFromSmiles(smiles))
        if AllChem.EmbedMolecule(molecule, randomSeed=7127) != 0 or AllChem.UFFOptimizeMolecule(molecule) != 0:
            raise RuntimeError(f"Geometry preparation failed for {name}")
        geometry = molecule.GetConformer().GetPositions()
        numbers = [atom.GetAtomicNum() for atom in molecule.GetAtoms()]
        hydrogen = numbers.index(1)
        parent = molecule.GetAtomWithIdx(hydrogen).GetNeighbors()[0].GetIdx()
        direction = geometry[hydrogen] - geometry[parent]
        scale = (covalent_radii[1] + covalent_radii[numbers[parent]]) / np.linalg.norm(direction)
        for ratio in (None, 0.72, 0.85, 1.15, 1.25):
            coordinates = geometry.copy()
            if ratio is not None:
                coordinates[hydrogen] = geometry[parent] + direction * scale * ratio
            document = {"schema_version": 1, "coordinate_unit": "angstrom", "charge": 0, "spin": 0,
                        "atoms": [{"atomic_number": z, "position_angstrom": xyz.tolist()}
                                  for z, xyz in zip(numbers, coordinates, strict=True)]}
            request = {"schema_version": 1, "job_id": str(uuid.uuid4()), "input_hash": "0" * 64,
                       "document": document, "settings": settings,
                       **{key: settings[key] for key in ("method", "basis", "max_cycle", "conv_tol")}}
            with contextlib.redirect_stdout(sys.stderr):
                prediction = evaluate_document(document, small, None, medium)
                _, gradient, _, _ = evaluate(request, lambda _: None)
            reference = -gradient * units.Hartree / units.Bohr
            errors = np.linalg.norm(np.asarray(prediction["forces_ev_per_angstrom"]) - reference, axis=1)
            diagnostic = prediction["diagnostics"]
            disagreement = diagnostic["disagreement"]["max_force_ev_per_angstrom"]
            rows.append({"molecule": name, "distance_ratio": ratio,
                         "geometry_warning": bool(diagnostic["minimum_ratio"] < 0.80 or diagnostic["connectivity_ratio"] > 1.20),
                         "disagreement": disagreement, "max_reference_difference": float(np.max(errors)),
                         "rms_reference_difference": float(np.sqrt(np.mean(errors**2)))})
    correlation = spearmanr([row["disagreement"] for row in rows], [row["max_reference_difference"] for row in rows])
    confusion = {"true_positive": 0, "false_positive": 0, "true_negative": 0, "false_negative": 0}
    for row in rows:
        warning = row["geometry_warning"] or row["disagreement"] > 0.50
        high_error = row["max_reference_difference"] > 0.50
        confusion[("true" if warning == high_error else "false") + ("_positive" if warning else "_negative")] += 1
    print(json.dumps({"unit": "eV/angstrom", "reference_settings": settings,
                      "pyscf_version": pyscf.__version__, "rdkit_version": rdBase.rdkitVersion,
                      "mace_version": prediction["mace_version"], "torch_version": prediction["torch_version"],
                      "small_sha256": prediction["scope"]["checkpoint_sha256"],
                      "medium_sha256": diagnostic["disagreement"]["checkpoint_sha256"],
                      "reference_difference_cutoff": 0.50, "spearman": float(correlation.statistic),
                      "combined_warning_confusion": confusion, "structures": rows}, indent=2, allow_nan=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("small", type=Path)
    parser.add_argument("medium", type=Path)
    args = parser.parse_args()
    main(args.small, args.medium)
