"""Optimize an isolated minimum and compute harmonic modes in implicit solvent."""
import hashlib
import json
import math

from .calculation import atomic_json, evaluate, validate_request
from .thermochemistry import MODELS, MODE_CUTOFF, PCM_GRID, PCM_PROBE, PCM_SCALE, SOLVENTS


def run(input_path, directory, *, model, medium, steps, on_step=None):
    import ase
    import h5py
    import numpy as np
    import pyscf
    from ase import Atoms, units
    from ase.calculators.calculator import Calculator, all_changes
    from ase.optimize import BFGS
    from pyscf.hessian import thermo

    if model not in MODELS or medium not in SOLVENTS or not 1 <= steps <= 100:
        raise ValueError("Invalid solution settings")
    with input_path.open("rb") as stream:
        source = stream.read(65537)
    if len(source) > 65536:
        raise ValueError("Oversized calculation input")
    request = json.loads(source)
    validate_request(request)
    document = request["document"]
    if (not 2 <= len(document["atoms"]) <= 10 or request["method"] != "RHF"
            or any(request.get(key) for key in (
                "optimize", "neb", "transition", "thermochemistry", "refinement", "dynamics"
            ))):
        raise ValueError("Solution optimization requires a gas-phase RHF single-point input with 2–10 atoms")

    class SolutionCalculator(Calculator):
        implemented_properties = ["energy", "forces"]

        def calculate(self, atoms=None, properties=("energy",), system_changes=all_changes):
            super().calculate(atoms, properties, system_changes)
            geometry = [
                {**atom, "position_angstrom": position}
                for atom, position in zip(document["atoms"], self.atoms.positions.tolist(), strict=True)
            ]
            image = {**request, "document": {**document, "atoms": geometry}}
            solvers = []
            energy, gradient, _, _ = evaluate(
                image, lambda _: None, on_converged=solvers.append,
                solvent={"model": model, "medium": medium},
            )
            self.solver = solvers[0]
            self.results = {"energy": energy * units.Hartree,
                            "forces": -gradient * units.Hartree / units.Bohr}

    pyscf.lib.num_threads(1)
    atoms = Atoms(numbers=[a["atomic_number"] for a in document["atoms"]],
                  positions=[a["position_angstrom"] for a in document["atoms"]])
    atoms.calc = SolutionCalculator()
    force_limit = 0.01
    maxstep = 0.1
    directory.mkdir(parents=True, exist_ok=False)
    with BFGS(atoms, logfile=None, maxstep=maxstep) as optimizer:
        if on_step is not None:
            optimizer.attach(lambda: on_step(optimizer.nsteps))
        optimizer.run(fmax=force_limit, steps=steps)
        completed = optimizer.nsteps
    force = float(np.linalg.norm(atoms.get_forces(), axis=1).max())
    if force >= force_limit:
        raise ArithmeticError("solution_limit")
    energy = atoms.get_potential_energy() / units.Hartree
    solver = atoms.calc.solver
    if on_step is not None:
        on_step(steps + 1)
    hessian = solver.Hessian().kernel()
    if hessian.shape != (len(atoms), len(atoms), 3, 3) or not np.isfinite(hessian).all():
        raise ArithmeticError("Invalid PCM Hessian")
    modes = thermo.harmonic_analysis(solver.mol, hessian, imaginary_freq=True)
    frequencies = [float(-abs(mode.imag) if mode.imag else mode.real)
                   for mode in modes["freq_wavenumber"]]
    if not frequencies or not all(map(math.isfinite, frequencies)):
        raise ArithmeticError("Invalid PCM frequencies")
    report = {
        "schema_version": 1, "input_sha256": hashlib.sha256(source).hexdigest(),
        "model": MODELS[model], "medium": medium, "dielectric": SOLVENTS[medium],
        "lebedev_order": PCM_GRID, "vdw_scale": PCM_SCALE, "r_probe": PCM_PROBE,
        "pyscf_version": pyscf.__version__, "ase_version": ase.__version__,
        "optimizer": "ASE BFGS", "step_limit": steps, "steps": completed,
        "force_limit_ev_per_angstrom": force_limit, "maxstep_angstrom": maxstep,
        "max_force_ev_per_angstrom": force, "electronic_hartree": float(energy),
        "frequencies_cm1": frequencies, "minimum_confirmed": all(mode > MODE_CUTOFF for mode in frequencies),
    }
    temporary = directory / "solution.partial.h5"
    try:
        with h5py.File(temporary, "x") as artifact:
            artifact.attrs["report_json"] = json.dumps(report, allow_nan=False)
            artifact.create_dataset("input_json", data=source.decode("utf-8"))
            artifact.create_dataset("atomic_numbers", data=atoms.numbers)
            artifact.create_dataset("positions", data=atoms.positions).attrs["unit"] = "angstrom"
            artifact.create_dataset("hessian", data=hessian).attrs["unit"] = "hartree/bohr^2"
            artifact.create_dataset("frequencies", data=frequencies).attrs["unit"] = "cm^-1"
        temporary.replace(directory / "solution.h5")
        atomic_json(directory / "solution-result.json", report)
    finally:
        temporary.unlink(missing_ok=True)
    return report
