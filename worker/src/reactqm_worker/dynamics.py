"""Bounded gas-phase Born–Oppenheimer dynamics with PySCF gradients."""
import hashlib
import json
import math

from .calculation import atomic_json, evaluate, validate_request


def run(input_path, directory, *, steps, timestep_fs, temperature_k, seed, ensemble, on_step=None):
    import ase
    import h5py
    import numpy as np
    import pyscf
    from ase import Atoms, units
    from ase.calculators.calculator import Calculator, all_changes
    from ase.md.langevin import Langevin
    from ase.md.verlet import VelocityVerlet

    if not (1 <= steps <= 500 and 0.05 <= timestep_fs <= 2
            and 1 <= temperature_k <= 2000 and 0 <= seed < 2**32
            and ensemble in ("nve", "langevin")):
        raise ValueError("Invalid dynamics settings")
    with input_path.open("rb") as stream:
        source = stream.read(65537)
    if len(source) > 65536:
        raise ValueError("Oversized calculation input")
    request = json.loads(source)
    validate_request(request)
    document = request["document"]
    if (not 2 <= len(document["atoms"]) <= 10
            or request["method"] not in ("RHF", "UHF", "RKS", "UKS")
            or any(request.get(key) for key in (
                "solvent", "optimize", "neb", "transition", "thermochemistry", "refinement"
            ))):
        raise ValueError("Dynamics requires 2–10 atoms in the gas phase")

    class PySCFCalculator(Calculator):
        implemented_properties = ["energy", "forces"]

        def calculate(self, atoms=None, properties=("energy",), system_changes=all_changes):
            super().calculate(atoms, properties, system_changes)
            geometry = [
                {**atom, "position_angstrom": position}
                for atom, position in zip(document["atoms"], self.atoms.positions.tolist(), strict=True)
            ]
            energy, gradient, _, _ = evaluate(
                {**request, "document": {**document, "atoms": geometry}}, lambda _: None
            )
            self.results = {
                "energy": energy * units.Hartree,
                "forces": -gradient * units.Hartree / units.Bohr,
            }

    pyscf.lib.num_threads(1)
    atoms = Atoms(numbers=[a["atomic_number"] for a in document["atoms"]],
                  positions=[a["position_angstrom"] for a in document["atoms"]])
    atoms.calc = PySCFCalculator()
    rng = np.random.default_rng(seed)
    masses = atoms.get_masses()[:, None]
    atoms.set_momenta(rng.standard_normal(atoms.positions.shape)
                      * np.sqrt(masses * units.kB * temperature_k))
    timestep = timestep_fs * units.fs
    friction_per_fs = 0.02
    if ensemble == "nve":
        integrator = VelocityVerlet(atoms, timestep)
    else:
        integrator = Langevin(atoms, timestep, temperature_K=temperature_k,
                              friction=friction_per_fs / units.fs, fixcm=False, rng=rng)

    directory.mkdir(parents=True, exist_ok=False)
    identity = {
        "schema_version": 1,
        "input_sha256": hashlib.sha256(source).hexdigest(),
        "ensemble": ensemble,
        "steps": steps,
        "timestep_fs": timestep_fs,
        "temperature_k": temperature_k,
        "seed": seed,
        "friction_per_fs": friction_per_fs if ensemble == "langevin" else None,
        "ase_version": ase.__version__,
        "numpy_version": np.__version__,
        "pyscf_version": pyscf.__version__,
        "libxc_version": pyscf.dft.libxc.__version__ if request["method"] in ("RKS", "UKS") else None,
    }
    energies = []
    temperatures = []
    temporary = directory / "trajectory.partial.h5"
    try:
        with integrator, h5py.File(temporary, "x") as artifact:
            artifact.attrs["provenance_json"] = json.dumps(identity, allow_nan=False)
            artifact.create_dataset("input_json", data=source.decode("utf-8"))
            artifact.create_dataset("atomic_numbers", data=atoms.numbers)
            positions = artifact.create_dataset("positions_angstrom", (steps + 1, len(atoms), 3), dtype="f8")
            momenta = artifact.create_dataset("momenta_ase", positions.shape, dtype="f8")
            momenta.attrs["unit"] = "sqrt(eV * atomic mass unit)"
            for frame in range(steps + 1):
                if frame:
                    integrator.run(1)
                potential = atoms.get_potential_energy()
                kinetic = atoms.get_kinetic_energy()
                temperature = atoms.get_temperature()
                if not (all(map(math.isfinite, (potential, kinetic, temperature)))
                        and np.isfinite(atoms.positions).all() and np.isfinite(atoms.get_momenta()).all()):
                    raise ValueError("Non-finite trajectory")
                positions[frame] = atoms.positions
                momenta[frame] = atoms.get_momenta()
                energies.append((potential, kinetic))
                temperatures.append(temperature)
                if on_step is not None:
                    on_step(frame)
            artifact.create_dataset("time_fs", data=np.arange(steps + 1) * timestep_fs)
            artifact.create_dataset("potential_ev", data=np.asarray(energies)[:, 0])
            artifact.create_dataset("kinetic_ev", data=np.asarray(energies)[:, 1])
            artifact.create_dataset("temperature_k", data=temperatures)
        total = np.sum(energies, axis=1)
        summary = {
            **identity,
            "frames": steps + 1,
            "initial_temperature_k": temperatures[0],
            "mean_temperature_k": float(np.mean(temperatures[1:])),
            "nve_final_energy_change_ev": float(total[-1] - total[0]) if ensemble == "nve" else None,
            "nve_max_energy_deviation_ev": float(np.max(np.abs(total - total[0]))) if ensemble == "nve" else None,
        }
        temporary.replace(directory / "trajectory.h5")
        atomic_json(directory / "dynamics-result.json", summary)
        return summary
    finally:
        temporary.unlink(missing_ok=True)
