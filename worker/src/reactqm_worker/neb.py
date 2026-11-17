"""ASE elastic band using independent PySCF calculators per image."""
import hashlib
import json
from dataclasses import dataclass

from .calculation import atomic_json, evaluate


@dataclass
class Band:
    summary: dict
    forces: object


def run(request, directory, emit):
    import ase
    from ase import Atoms, units
    from ase.calculators.calculator import Calculator, all_changes
    from ase.mep import NEB
    from ase.optimize import FIRE
    import numpy as np

    document = request["document"]
    path = request["neb"]
    input_bytes = (directory / "input.json").read_bytes()
    input_digest = hashlib.sha256(input_bytes).hexdigest()

    class PySCFImage(Calculator):
        implemented_properties = ["energy", "forces"]

        def calculate(self, atoms=None, properties=("energy",), system_changes=all_changes):
            super().calculate(atoms, properties, system_changes)
            geometry = [{**atom, "position_angstrom": position} for atom, position in
                        zip(document["atoms"], self.atoms.positions.tolist(), strict=True)]
            image_request = {key: value for key, value in request.items() if key != "neb"}
            image_request["document"] = {**document, "atoms": geometry}
            energy, gradient, _, _ = evaluate(image_request, lambda _: None)
            self.results = {"energy": energy * units.Hartree, "forces": -gradient * units.Hartree / units.Bohr}

    first = np.array([atom["position_angstrom"] for atom in document["atoms"]])
    last = np.array(path["product"])
    coordinates = path.get("restart") or np.linspace(first, last, 5).tolist()
    images = [Atoms(numbers=[atom["atomic_number"] for atom in document["atoms"]], positions=xyz)
              for xyz in coordinates]
    for image in images:
        image.calc = PySCFImage()
    neb = NEB(images, climb=path["climb"], method="improvedtangent", k=0.1)
    checkpoint = directory / "neb-checkpoint.json"
    with FIRE(neb, maxstep=0.1, logfile=None) as optimizer:
        def record(notify=True):
            projected = np.linalg.norm(neb.get_forces(), axis=1).max()
            stats = [{"energy_hartree": float(image.get_potential_energy() / units.Hartree),
                      "max_force_ev_per_angstrom": float(np.linalg.norm(image.get_forces(), axis=1).max())}
                     for image in images]
            summary = {"steps": optimizer.nsteps, "ase_version": ase.__version__, "climb": path["climb"],
                       "images": stats, "max_neb_force_ev_per_angstrom": float(projected),
                       "positions": [image.positions.tolist() for image in images]}
            atomic_json(checkpoint, {"input_sha256": input_digest, "summary": summary})
            if notify:
                metrics = {key: value for key, value in summary.items() if key != "positions"}
                with (directory / "neb-progress.jsonl").open("a", encoding="utf-8") as log:
                    log.write(json.dumps(metrics, allow_nan=False) + "\n")
                emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": optimizer.nsteps})
            return summary

        optimizer.attach(record)
        converged = optimizer.run(fmax=0.05, steps=50)
        summary = record(notify=False)
    if not converged or summary["max_neb_force_ev_per_angstrom"] >= 0.05:
        raise ArithmeticError("neb_limit")
    return Band(summary, np.asarray([image.get_forces() for image in images]))
