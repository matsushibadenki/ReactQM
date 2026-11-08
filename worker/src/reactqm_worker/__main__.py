"""Diagnostics and the versioned local calculation protocol."""
import argparse
import importlib
import importlib.metadata
import json
from pathlib import Path
import sqlite3
import tempfile


PACKAGES = {"numpy": "numpy", "scipy": "scipy", "pyscf": "pyscf", "ase": "ase", "rdkit": "rdkit", "h5py": "h5py"}


def doctor() -> dict:
    versions = {}
    for package, module in PACKAGES.items():
        importlib.import_module(module)
        versions[package] = importlib.metadata.version(package)
    return {"status": "ok", "packages": versions, "sqlite": sqlite3.sqlite_version}


def smoke() -> dict:
    import h5py
    import numpy as np
    from ase import Atoms
    from pyscf import gto, lib, scf
    from rdkit import Chem

    lib.num_threads(1)
    molecule = gto.M(atom="H 0 0 0; H 0 0 0.74", basis="sto-3g", unit="Angstrom", charge=0, spin=0, verbose=0)
    calculation = scf.RHF(molecule)
    energy = float(calculation.kernel())
    # Small numerical regression for installation correctness, not accuracy of chemical predictions.
    if not calculation.converged or abs(energy - (-1.1167593074)) > 1e-6:
        raise RuntimeError(f"H2 RHF/STO-3G regression failed: {energy}")
    gradients = calculation.nuc_grad_method().kernel()
    if gradients.shape != (2, 3) or not np.isfinite(gradients).all():
        raise RuntimeError("Invalid nuclear gradient")
    if not np.allclose(gradients.sum(axis=0), 0, atol=1e-8):
        raise RuntimeError("Nuclear gradient fails translation invariance")
    atoms = Atoms("H2", positions=molecule.atom_coords(unit="Angstrom"))
    smiles = Chem.MolFromSmiles("CO")
    if smiles is None or smiles.GetNumAtoms() != 2 or len(atoms) != 2:
        raise RuntimeError("RDKit/ASE smoke test failed")
    with tempfile.TemporaryDirectory(prefix="reactqm-") as directory:
        path = Path(directory)
        with h5py.File(path / "result.h5", "w") as result:
            dataset = result.create_dataset("gradient", data=gradients)
            dataset.attrs["unit"] = "hartree/bohr"
        with h5py.File(path / "result.h5", "r") as result:
            if not np.array_equal(result["gradient"][:], gradients):
                raise RuntimeError("HDF5 roundtrip failed")
        with sqlite3.connect(path / "jobs.sqlite") as db:
            db.execute("CREATE TABLE results (energy_hartree REAL NOT NULL)")
            db.execute("INSERT INTO results VALUES (?)", (energy,))
            if db.execute("SELECT energy_hartree FROM results").fetchone()[0] != energy:
                raise RuntimeError("SQLite roundtrip failed")
    return {"status": "ok", "fixture": "H2", "method": "RHF", "basis": "sto-3g", "distance_angstrom": 0.74, "energy_hartree": energy, "converged": bool(calculation.converged), "gradient_unit": "hartree/bohr", "checks": ["PySCF energy", "PySCF gradient", "RDKit", "ASE", "HDF5", "SQLite"]}


def main() -> None:
    parser = argparse.ArgumentParser(description="ReactQM worker diagnostics")
    parser.add_argument("command", choices=["doctor", "smoke", "calculate", "valence"])
    parser.add_argument("--output-dir", type=Path)
    args = parser.parse_args()
    if args.command == "valence":
        from .valence import serve
        raise SystemExit(serve())
    if args.command == "calculate":
        if args.output_dir is None:
            parser.error("calculate requires --output-dir")
        from .calculation import serve
        raise SystemExit(serve(args.output_dir))
    try:
        result = doctor() if args.command == "doctor" else smoke()
        print(json.dumps(result, ensure_ascii=False, allow_nan=False))
    except Exception as error:
        print(json.dumps({"status": "error", "error": str(error)}, ensure_ascii=False))
        raise SystemExit(1) from error


if __name__ == "__main__":
    main()
