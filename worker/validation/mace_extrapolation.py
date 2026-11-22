"""Independent geometry challenge for the pinned OFF23 small and medium models."""
import contextlib
import json
from pathlib import Path
import sys

from ase.data import covalent_radii
import numpy as np
from rdkit import Chem
from rdkit.Chem import AllChem

from reactqm_worker.mace_potential import evaluate_document


def main(small, medium):
    for name, smiles in (("methane", "C"), ("ammonia", "N"), ("methanol", "CO"), ("ethane", "CC")):
        molecule = Chem.AddHs(Chem.MolFromSmiles(smiles))
        if AllChem.EmbedMolecule(molecule, randomSeed=2718) != 0 or AllChem.UFFOptimizeMolecule(molecule) != 0:
            raise RuntimeError(f"Geometry preparation failed for {name}")
        positions = molecule.GetConformer().GetPositions()
        numbers = [atom.GetAtomicNum() for atom in molecule.GetAtoms()]
        hydrogen = numbers.index(1)
        parent = molecule.GetAtomWithIdx(hydrogen).GetNeighbors()[0].GetIdx()
        direction = positions[hydrogen] - positions[parent]
        radii = covalent_radii[1] + covalent_radii[numbers[parent]]
        for variant, ratio in (("baseline", None), ("compressed", 0.72), ("stretched", 1.25)):
            coordinates = positions.copy()
            if ratio is not None:
                coordinates[hydrogen] = positions[parent] + direction / np.linalg.norm(direction) * radii * ratio
            document = {"schema_version": 1, "coordinate_unit": "angstrom", "charge": 0, "spin": 0,
                        "atoms": [{"atomic_number": z, "position_angstrom": xyz.tolist()}
                                  for z, xyz in zip(numbers, coordinates, strict=True)]}
            with contextlib.redirect_stdout(sys.stderr):
                prediction = evaluate_document(document, small, None, medium)
            report = prediction["diagnostics"]
            geometry_warning = report["minimum_ratio"] < 0.80 or report["connectivity_ratio"] > 1.20
            assert geometry_warning == (variant != "baseline"), (name, variant, report)
            print(json.dumps({"molecule": name, "variant": variant, **report}, allow_nan=False), flush=True)


if __name__ == "__main__":
    main(*map(Path, sys.argv[1:]))
