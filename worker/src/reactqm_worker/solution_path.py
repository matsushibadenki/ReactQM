"""Reoptimize an atom-mapped reaction in PCM and verify its saddle by IRC."""
import hashlib
import json
import math
import tempfile
from pathlib import Path

from .calculation import atomic_json, validate_request
from .solution import run as optimize_minimum
from .thermochemistry import MODELS, MODE_CUTOFF, SOLVENTS
from .transition import run as optimize_saddle


def run(input_path, directory, *, model, medium, steps, on_phase=lambda _: None):
    import h5py
    import pyscf
    from pyscf.data import nist

    with input_path.open("rb") as stream:
        source = stream.read(65537)
    if len(source) > 65536:
        raise ValueError("Oversized transition input")
    request = json.loads(source)
    validate_request(request)
    path = request.get("transition")
    document = request["document"]
    if (not path or request["method"] != "RHF" or not 2 <= len(document["atoms"]) <= 10
            or model not in MODELS or medium not in SOLVENTS or not 1 <= steps <= 100
            or any(request.get(key) for key in ("solution", "dynamics"))):
        raise ValueError("Expected a mapped RHF transition input and valid PCM settings")

    pyscf.lib.num_threads(1)
    directory.mkdir(parents=True, exist_ok=False)
    partial = directory / "path.partial.h5"
    try:
        with tempfile.TemporaryDirectory(dir=directory) as scratch, h5py.File(partial, "x") as output:
            output.create_dataset("input_json", data=source.decode("utf-8"))
            endpoints = {}
            relaxed = {}
            for phase, name in enumerate(("reactant", "product")):
                on_phase(phase)
                endpoint = {key: value for key, value in request.items() if key != "transition"}
                endpoint["document"] = {**document, "atoms": [
                    {**atom, "position_angstrom": position}
                    for atom, position in zip(document["atoms"], path[name], strict=True)
                ]}
                endpoint_input = Path(scratch) / f"{name}.json"
                atomic_json(endpoint_input, endpoint)
                target = Path(scratch) / name
                endpoints[name] = optimize_minimum(endpoint_input, target, model=model, medium=medium, steps=steps)
                with h5py.File(target / "solution.h5", "r") as artifact:
                    group = output.create_group(name)
                    group.attrs["report_json"] = artifact.attrs["report_json"]
                    for key in ("positions", "hessian", "frequencies"):
                        artifact.copy(key, group)
                    relaxed[name] = group["positions"][:].tolist()

            seeded = {**request, "transition": {**path, **relaxed}}
            saddle, trajectories, hessian, energy, _, positions, _ = optimize_saddle(
                seeded, lambda event: on_phase(event["cycle"] + 2), solvent={"model": model, "medium": medium},
            )
            modes = saddle["frequencies_cm1"]
            if not modes or not all(map(math.isfinite, modes)):
                raise ArithmeticError("Invalid saddle frequencies")
            matches = {branch["match"] for branch in saddle["branches"] if branch["converged"]}
            verified = (all(item["minimum_confirmed"] for item in endpoints.values())
                        and saddle["imaginary_count"] == 1
                        and all(abs(mode) > MODE_CUTOFF for mode in modes)
                        and matches == {"reactant", "product"})
            barriers = None
            if verified:
                barriers = {
                    direction: (energy - endpoints[name]["electronic_hartree"]) * nist.HARTREE2J * nist.AVOGADRO / 1000
                    for direction, name in (("forward", "reactant"), ("reverse", "product"))
                }
                if any(value <= 0 for value in barriers.values()):
                    verified = False
                    barriers = None
            saddle["electronic_hartree"] = energy
            group = output.create_group("saddle")
            group.create_dataset("positions", data=positions).attrs["unit"] = "angstrom"
            group.create_dataset("hessian", data=hessian).attrs["unit"] = "hartree/bohr^2"
            group.create_dataset("frequencies", data=modes).attrs["unit"] = "cm^-1"
            for direction, trajectory in zip(("irc_forward", "irc_reverse"), trajectories):
                group.create_dataset(direction, data=trajectory).attrs["unit"] = "angstrom"
            report = {
                "schema_version": 1, "input_sha256": hashlib.sha256(source).hexdigest(),
                "model": MODELS[model], "medium": medium, "dielectric": SOLVENTS[medium],
                "reactant": endpoints["reactant"], "product": endpoints["product"],
                "saddle": saddle, "path_verified": verified, "electronic_barriers_kj_mol": barriers,
            }
            output.attrs["report_json"] = json.dumps(report, allow_nan=False)
        partial.replace(directory / "path.h5")
        atomic_json(directory / "path-result.json", report)
    finally:
        partial.unlink(missing_ok=True)
    return report
