"""Diagnostics and the versioned local calculation protocol."""
import argparse
import contextlib
import sys
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
    try:
        from tblite.library import get_version
        versions["tblite"] = ".".join(map(str, get_version()))
    except (ImportError, OSError):
        versions["tblite"] = None
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
    parser.add_argument("command", choices=["doctor", "smoke", "calculate", "valence", "structure", "optimize", "field", "dynamics", "solution", "solution-path", "stoichiometry", "mace", "mace-scope", "mace-reference-batch"])
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--input", type=Path)
    parser.add_argument("--fmax", type=float, default=0.05)
    parser.add_argument("--steps", type=int, default=50)
    parser.add_argument("--resume", action="store_true")
    parser.add_argument("--timestep-fs", type=float, default=0.5)
    parser.add_argument("--temperature-k", type=float, default=300)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--ensemble", choices=["nve", "langevin"], default="nve")
    parser.add_argument("--solvent-model", choices=["c_pcm", "ief_pcm"], default="c_pcm")
    parser.add_argument("--solvent", choices=["water", "acetonitrile"], default="water")
    parser.add_argument("--model-path", type=Path)
    args = parser.parse_args()
    if args.command == "mace-reference-batch":
        if args.input is None:
            parser.error("mace-reference-batch requires --input")
        from .active_learning import run

        try:
            with contextlib.redirect_stdout(sys.stderr):
                result = run(args.input)
            print(json.dumps(result, allow_nan=False))
        except Exception as error:
            parser.exit(1, f"MACE reference batch failed: {error}\n")
        return
    if args.command in ("mace", "mace-scope"):
        from .mace_potential import evaluate, scope

        if args.command == "mace-scope":
            print(json.dumps(scope()))
            return
        if args.input is None or args.model_path is None:
            parser.error("mace requires --input and --model-path")
        try:
            with contextlib.redirect_stdout(sys.stderr):
                result = evaluate(args.input, args.model_path)
            print(json.dumps(result, allow_nan=False))
        except Exception as error:
            parser.exit(1, f"MACE calculation failed: {error}\n")
        return
    if args.command == "stoichiometry":
        if args.input is None:
            parser.error("stoichiometry requires --input")
        from .stoichiometry import compare

        try:
            print(json.dumps(compare(args.input), allow_nan=False))
        except Exception as error:
            parser.exit(1, f"Stoichiometry calculation failed: {error}\n")
        return
    if args.command in ("solution", "solution-path"):
        if args.input is None or args.output_dir is None or args.resume:
            parser.error(f"{args.command} requires --input and a fresh --output-dir")
        if args.command == "solution-path":
            from .solution_path import run
        else:
            from .solution import run

        try:
            result = run(args.input, args.output_dir, model=args.solvent_model,
                         medium=args.solvent, steps=args.steps)
            print(json.dumps(result, allow_nan=False))
        except Exception as error:
            parser.exit(1, f"Solution calculation failed: {error}\n")
        return
    if args.command == "dynamics":
        if args.input is None or args.output_dir is None or args.resume:
            parser.error("dynamics requires --input and a fresh --output-dir")
        from .dynamics import run
        try:
            result = run(args.input, args.output_dir, steps=args.steps,
                         timestep_fs=args.timestep_fs, temperature_k=args.temperature_k,
                         seed=args.seed, ensemble=args.ensemble)
            print(json.dumps(result, allow_nan=False))
        except Exception as error:
            parser.exit(1, f"Dynamics failed: {error}\n")
        return
    if args.command == "optimize":
        if args.input is None or args.output_dir is None:
            parser.error("optimize requires --input and --output-dir")
        from .optimization import run_cli
        raise SystemExit(run_cli(args.input, args.output_dir, args.fmax, args.steps, args.resume))
    if args.command == "field":
        from .fields import serve
        raise SystemExit(serve())
    if args.command == "structure":
        from .structure import serve
        raise SystemExit(serve())
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
