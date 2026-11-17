"""Version 1 JSON-lines protocol for bounded HF/DFT calculation jobs."""
import contextlib
import json
import math
import os
from pathlib import Path
import sys
import time
import uuid

from . import screening


def validate_request(request: dict) -> None:
    if request.get("schema_version") != 1:
        raise ValueError("Unsupported job schema or method")
    uuid.UUID(request["job_id"])
    fingerprint = request["input_hash"]
    if len(fingerprint) != 64 or any(c not in "0123456789abcdef" for c in fingerprint):
        raise ValueError("Invalid input hash")
    path = request.get("neb")
    if path is not None:
        if request.get("optimize") or request["method"] == screening.METHOD:
            raise ValueError("NEB requires a PySCF single-point adapter")
        original = request["document"]["atoms"]
        if not 2 <= len(original) <= 16 or type(path["climb"]) is not bool:
            raise ValueError("Invalid NEB input")
        start = [atom["position_angstrom"] for atom in original]
        frames = path.get("restart", [start, path["product"]])
        if len(frames) != (5 if "restart" in path else 2) or frames[0] != start or frames[-1] != path["product"]:
            raise ValueError("Invalid NEB endpoints")
        for frame in frames:
            if len(frame) != len(original):
                raise ValueError("Invalid NEB image")
            image = {key: value for key, value in request.items() if key != "neb"}
            image["document"] = {**request["document"], "atoms": [
                {**atom, "position_angstrom": position} for atom, position in zip(original, frame, strict=True)
            ]}
            validate_request(image)
    transition = request.get("transition")
    if transition is not None:
        original = request["document"]["atoms"]
        if path is not None or request.get("optimize") or request["method"] == screening.METHOD or not 2 <= len(original) <= 16:
            raise ValueError("Invalid transition input")
        uuid.UUID(transition["source_job"])
        if type(transition["seed_image"]) is not int or not 1 <= transition["seed_image"] <= 3:
            raise ValueError("Invalid transition seed")
        for side in ("reactant", "product"):
            image = {key: value for key, value in request.items() if key != "transition"}
            image["document"] = {**request["document"], "atoms": [
                {**atom, "position_angstrom": coordinates} for atom, coordinates in zip(original, transition[side], strict=True)
            ]}
            validate_request(image)
    thermal = request.get("thermochemistry")
    if thermal is not None:
        original = request["document"]["atoms"]
        if path is not None or transition is not None or request.get("optimize") or request["method"] == screening.METHOD or not 2 <= len(original) <= 16:
            raise ValueError("Invalid thermochemistry input")
        uuid.UUID(thermal["source_job"])
        if type(thermal["temperature_kelvin"]) not in (int, float) or not 1 <= thermal["temperature_kelvin"] <= 2000:
            raise ValueError("Invalid temperature")
        if thermal.get("standard_state", "atmosphere") not in ("atmosphere", "molar"):
            raise ValueError("Invalid standard state")
        if "solvent" in thermal:
            from .thermochemistry import MODELS, SOLVENTS
            solvent = thermal["solvent"]
            if set(solvent) != {"model", "medium"} or solvent["model"] not in MODELS or solvent["medium"] not in SOLVENTS:
                raise ValueError("Invalid PCM conditions")
        for side in ("reactant", "product"):
            image = {key: value for key, value in request.items() if key != "thermochemistry"}
            image["document"] = {**request["document"], "atoms": [
                {**atom, "position_angstrom": coordinates} for atom, coordinates in zip(original, thermal[side], strict=True)
            ]}
            validate_request(image)
    refinement = request.get("refinement")
    if refinement is not None:
        document = request["document"]
        original = document["atoms"]
        if (path is not None or transition is not None or thermal is not None or request.get("optimize")
                or request["method"] != "RHF" or document["spin"] != 0 or not 2 <= len(original) <= 10
                or refinement["method"] not in ("mp2", "ccsdt")):
            raise ValueError("Invalid refinement input")
        baseline = refinement["baseline"]
        for field in ("source_job", "path_job"):
            uuid.UUID(baseline[field])
        if len(baseline["energies_hartree"]) != 3 or not all(math.isfinite(x) for x in baseline["energies_hartree"]):
            raise ValueError("Invalid baseline energies")
        for side in ("reactant", "product"):
            image = {key: value for key, value in request.items() if key != "refinement"}
            image["document"] = {**document, "atoms": [
                {**atom, "position_angstrom": coordinates} for atom, coordinates in zip(original, refinement[side], strict=True)
            ]}
            validate_request(image)
    is_screening = request["method"] == screening.METHOD
    max_cycles = screening.MAX_ITERATIONS if is_screening else 50
    if type(request["max_cycle"]) is not int or not 1 <= request["max_cycle"] <= max_cycles:
        raise ValueError("Invalid cycle limit")
    tolerance = request["conv_tol"]
    if type(tolerance) not in (int, float) or (tolerance != 0 if is_screening else not 1e-12 <= tolerance <= 1e-6):
        raise ValueError("Invalid convergence threshold")
    settings = request.get("settings")
    if settings is None:
        if request["method"] != "RHF" or request["basis"] != "sto-3g":
            raise ValueError("Legacy settings")
    else:
        if set(settings) != {"method", "basis", "xc", "grid_level", "max_cycle", "conv_tol"}:
            raise ValueError("Settings schema")
        if any(settings[k] != request[k] for k in ["method", "basis", "max_cycle", "conv_tol"]):
            raise ValueError("Mismatched settings")
        allowed_bases = ["gfn2"] if is_screening else ["sto-3g", "6-31g", "def2-svp"]
        if settings["basis"] not in allowed_bases:
            raise ValueError("Basis")
        if is_screening:
            if (settings["xc"] is not None or settings["grid_level"] is not None
                    or settings["max_cycle"] != screening.MAX_ITERATIONS or request.get("optimize", False)):
                raise ValueError("GFN2-xTB single-point settings")
        elif settings["method"] in ("RHF", "UHF"):
            if settings["xc"] is not None or settings["grid_level"] is not None:
                raise ValueError("HF settings")
        elif settings["method"] in ("RKS", "UKS"):
            if settings["xc"] not in ["PBE", "PBE0", "B3LYP"] or type(settings["grid_level"]) is not int or settings["grid_level"] not in [3,5]:
                raise ValueError("DFT settings")
        else:
            raise ValueError("Method")
    document = request["document"]
    if document["schema_version"] != 1 or document["coordinate_unit"] != "angstrom":
        raise ValueError("Unsupported document schema or unit")
    if type(document["charge"]) is not int or abs(document["charge"]) > 64 or type(document["spin"]) is not int:
        raise ValueError("Invalid charge or spin")
    atoms = document["atoms"]
    if not 1 <= len(atoms) <= 64:
        raise ValueError("Atom limit exceeded")
    max_element = 86 if is_screening else 10
    for index, atom in enumerate(atoms):
        number, coordinates = atom["atomic_number"], atom["position_angstrom"]
        if type(number) is not int or not 1 <= number <= max_element:
            raise ValueError("Element outside the selected method range")
        if len(coordinates) != 3 or any(type(v) not in (int, float) or not math.isfinite(v) or abs(v) > 10000 for v in coordinates):
            raise ValueError("Invalid coordinates")
        for other in atoms[:index]:
            if sum((a-b)**2 for a, b in zip(coordinates, other["position_angstrom"])) < 0.01:
                raise ValueError("Overlapping atoms")
    electrons = sum(atom["atomic_number"] for atom in atoms) - document["charge"]
    spin = document["spin"]
    if electrons <= 0 or not 0 <= spin <= electrons or (electrons - spin) % 2:
        raise ValueError("Inconsistent electron count")
    if spin and is_screening:
        raise ValueError("GFN2-xTB screening requires a singlet")
    if spin and request["method"] in ("RHF", "RKS"):
        raise ValueError("Nonzero spin requires UHF or UKS")


def atomic_json(path: Path, value: dict) -> None:
    temporary = path.with_name(f".{uuid.uuid4()}.tmp")
    try:
        with temporary.open("x", encoding="utf-8") as stream:
            json.dump(value, stream, allow_nan=False)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def evaluate(request, emit, checkpoint=None, on_converged=None):
    """Return energy in hartree, gradient in hartree/bohr and coordinates in Å."""
    validate_request(request)
    import numpy as np
    from pyscf import gto, scf, dft

    document = request["document"]
    molecule = gto.M(atom=[(atom["atomic_number"], atom["position_angstrom"]) for atom in document["atoms"]],
                     unit="Angstrom", basis=request["basis"], charge=document["charge"], spin=document["spin"],
                     verbose=0, max_memory=512)
    settings = request.get("settings")
    method = request["method"]
    solver = {"RHF": scf.RHF, "UHF": scf.UHF, "RKS": dft.RKS, "UKS": dft.UKS}[method](molecule)
    if request["method"] in ("RKS", "UKS"):
        solver.xc = settings["xc"]
        solver.grids.level = settings["grid_level"]
    solver.max_cycle = request["max_cycle"]
    solver.conv_tol = request["conv_tol"]
    solver.chkfile = checkpoint
    solver.callback = lambda env: emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": int(env["cycle"]) + 1})
    energy = float(solver.kernel())
    if not solver.converged:
        raise ArithmeticError("not_converged")
    if not math.isfinite(energy):
        raise ValueError("Non-finite energy")
    gradients = solver.nuc_grad_method().kernel()
    if gradients.shape != (len(document["atoms"]), 3) or not np.isfinite(gradients).all():
        raise ValueError("Invalid gradient")
    spin_squared = float(solver.spin_square()[0]) if method in ("UHF", "UKS") else None
    if spin_squared is not None and (not math.isfinite(spin_squared) or spin_squared < -1e-10):
        raise ValueError("Invalid spin expectation")
    if spin_squared is not None:
        spin_squared = max(0.0, spin_squared)
    if on_converged is not None:
        on_converged(solver)
    return energy, gradients, molecule.atom_coords(unit="Angstrom"), spin_squared


def calculate(request: dict, output_dir: Path, emit) -> dict:
    validate_request(request)
    import h5py

    started = time.monotonic()
    output_dir.mkdir(parents=True, exist_ok=True)
    optimization = None
    band = None
    transition = None
    thermal = None
    refinement = None
    evaluation_request = request
    evaluation_emit = emit
    if request.get("optimize", False):
        from .optimization import optimize

        def progress(state):
            emit({"kind": "progress", "schema_version": 1,
                  "job_id": request["job_id"], "cycle": state["steps"]})

        output = optimize(request, output_dir, 0.05, 50, False, on_step=progress)
        if not output["converged"]:
            raise ArithmeticError("optimization_limit")
        positions = output["positions_angstrom"]
        evaluation_request = {**request, "document": {**request["document"], "atoms": [
            {**atom, "position_angstrom": position}
            for atom, position in zip(request["document"]["atoms"], positions, strict=True)
        ]}}
        evaluation_emit = lambda _: None
        optimization = {
            "steps": output["steps"], "max_force_ev_per_angstrom": output["max_force_ev_per_angstrom"],
            "ase_version": output["identity"]["ase_version"], "positions_angstrom": positions,
        }
    solver = []
    is_screening = request["method"] == screening.METHOD
    if request.get("refinement") is not None:
        import pyscf
        from .refinement import evaluate as evaluate_refinement

        params = request["refinement"]
        coordinates = [atom["position_angstrom"] for atom in request["document"]["atoms"]]
        geometries = [params["reactant"], params["product"], coordinates]
        results = []
        for cycle, geometry in enumerate(geometries):
            emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": cycle})
            results.append(evaluate_refinement(request, geometry))
        energies, references, triples, counts = zip(*results)
        refinement = {"method": params["method"], "baseline": params["baseline"],
                      "energies_hartree": energies, "reference_hartree": references,
                      "triples_hartree": triples, "orbital_count": counts[0]}
        energy = energies[2]
        spin_squared = gradients = None
        engine_version = pyscf.__version__
    elif request.get("thermochemistry") is not None:
        import pyscf
        from .thermochemistry import analyze, standard_shift

        params = request["thermochemistry"]
        thermal = {key: params[key] for key in ("source_job", "temperature_kelvin")}
        for key in ("standard_state", "solvent"):
            if key in params:
                thermal[key] = params[key]
        if params.get("standard_state") == "molar":
            thermal["standard_shift_hartree"] = standard_shift(params["temperature_kelvin"], "molar")
        coordinates = [atom["position_angstrom"] for atom in request["document"]["atoms"]]
        for cycle, name in enumerate(("reactant", "product", "saddle")):
            emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": cycle})
            species, gradients, spin_squared = analyze(request, params.get(name, coordinates), params["temperature_kelvin"], name == "saddle", params)
            thermal[name] = species
        energy = thermal["saddle"]["electronic_hartree"]
        engine_version = pyscf.__version__
    elif request.get("transition") is not None:
        import pyscf
        from .transition import run

        transition, trajectories, hessian, energy, gradients, coordinates, spin_squared = run(request, emit)
        transition.update({key: request["transition"][key] for key in ("source_job", "seed_image")})
        engine_version = pyscf.__version__
    elif request.get("neb") is not None:
        import pyscf
        from ase import units
        from .neb import run

        band = run(request, output_dir, emit)
        energy = band.summary["images"][0]["energy_hartree"]
        gradients = -band.forces[0] * units.Bohr / units.Hartree
        coordinates = band.summary["positions"][0]
        engine_version = pyscf.__version__
        spin_squared = None
    elif is_screening:
        energy, gradients, coordinates, engine_version, binding_version = screening.evaluate(request["document"])
        spin_squared = None
    else:
        import pyscf
        energy, gradients, coordinates, spin_squared = evaluate(evaluation_request, evaluation_emit, str(output_dir / "scf.chk"), on_converged=solver.append)
        engine_version = pyscf.__version__
    if optimization is not None:
        import numpy as np
        from ase import units

        max_force = float(np.linalg.norm(gradients, axis=1).max() * units.Hartree / units.Bohr)
        if max_force >= 0.05:
            raise ArithmeticError("not_converged")
        optimization["max_force_ev_per_angstrom"] = max_force
    settings = request.get("settings")
    temporary = output_dir / "result.h5.tmp"
    fields = []
    with h5py.File(temporary, "w") as artifact:
        artifact.attrs["schema_version"] = 1
        artifact.attrs["job_id"] = request["job_id"]
        artifact.attrs["input_hash"] = request["input_hash"]
        artifact.attrs["method"] = request["method"]
        artifact.attrs["basis"] = request["basis"]
        if is_screening:
            artifact.attrs["tblite_version"] = engine_version
            artifact.attrs["tblite_python_version"] = binding_version
            artifact.attrs["scc_max_iterations"] = request["max_cycle"]
            artifact.attrs["scc_accuracy"] = screening.ACCURACY
            artifact.attrs["electronic_temperature_kelvin"] = screening.TEMPERATURE_K
            artifact.attrs["environment"] = "gas"
            artifact.attrs["converged"] = True
        else:
            artifact.attrs["pyscf_version"] = engine_version
            artifact.attrs["max_cycle"] = request["max_cycle"]
            artifact.attrs["conv_tol"] = request["conv_tol"]
        if settings is not None:
            artifact.attrs["settings_json"] = json.dumps(settings, sort_keys=True)
        if request["method"] in ("RKS", "UKS"):
            from pyscf import dft
            artifact.attrs["libxc_version"] = dft.libxc.__version__
        artifact.attrs["charge"] = request["document"]["charge"]
        artifact.attrs["spin"] = request["document"]["spin"]
        artifact.attrs["multiplicity"] = request["document"]["spin"] + 1
        if spin_squared is not None:
            artifact.attrs["spin_squared"] = spin_squared
        if optimization is not None:
            artifact.attrs["optimizer"] = "ASE BFGS"
            artifact.attrs["ase_version"] = optimization["ase_version"]
            artifact.attrs["optimization_steps"] = optimization["steps"]
            artifact.attrs["max_force_ev_per_angstrom"] = optimization["max_force_ev_per_angstrom"]
        artifact.create_dataset("energy", data=energy).attrs["unit"] = "hartree"
        if gradients is not None:
            artifact.create_dataset("gradient", data=gradients).attrs["unit"] = "hartree/bohr"
        artifact.create_dataset("coordinates", data=coordinates).attrs["unit"] = "angstrom"
        if refinement is not None:
            group = artifact.create_group("refinement")
            group.attrs["method"] = refinement["method"]
            group.attrs["reference"] = "all-electron RHF"
            group.attrs["orbital_count"] = refinement["orbital_count"]
            group.attrs["baseline_json"] = json.dumps(refinement["baseline"])
            group.attrs["species_order"] = "reactant, product, saddle"
            if refinement["method"] == "ccsdt":
                group.attrs["ccsd_max_cycle"] = 50
                group.attrs["ccsd_conv_tol"] = 1e-8
                group.attrs["ccsd_conv_tol_normt"] = 1e-6
            for key in ("energies_hartree", "reference_hartree", "triples_hartree"):
                group.create_dataset(key, data=refinement[key]).attrs["unit"] = "hartree"
            group.create_dataset("geometries", data=geometries).attrs["unit"] = "angstrom"
        if thermal is not None:
            from .thermochemistry import PRESSURE, FMAX, MODE_CUTOFF, SOLVENTS, MODELS

            group = artifact.create_group("thermochemistry")
            for key in ("source_job", "temperature_kelvin"):
                group.attrs[key] = thermal[key]
            group.attrs["model"] = "ideal-gas RRHO, single species per structure"
            group.attrs["pressure_pa"] = PRESSURE
            group.attrs["fmax_ev_per_angstrom"] = FMAX
            group.attrs["standard_state"] = thermal.get("standard_state", "atmosphere")
            group.attrs["standard_shift_hartree"] = thermal.get("standard_shift_hartree", 0.0)
            if "solvent" in thermal:
                solvent = thermal["solvent"]
                group.attrs["pcm_model"] = MODELS[solvent["model"]]
                group.attrs["pcm_solvent"] = solvent["medium"]
                group.attrs["pcm_dielectric"] = SOLVENTS[solvent["medium"]]
                group.attrs["pcm_lebedev_order"] = 29
                group.attrs["pcm_vdw_scale"] = 1.2
                group.attrs["pcm_probe_radius_angstrom"] = 0.0
            group.attrs["mode_cutoff_cm1"] = MODE_CUTOFF
            for name in ("reactant", "product", "saddle"):
                item = thermal[name]
                species = group.create_group(name)
                species.create_dataset("frequencies", data=item["frequencies_cm1"]).attrs["unit"] = "cm^-1"
                for key in ("electronic_hartree", "max_force_ev_per_angstrom"):
                    species.attrs[key] = item[key]
                if "solvated_electronic_hartree" in item:
                    species.attrs["solvated_electronic_hartree"] = item["solvated_electronic_hartree"]
                if item["corrections"] is not None:
                    for key, value in item["corrections"].items():
                        species.attrs[key] = value
        if transition is not None:
            from .transition import FMAX, IRC_FMAX, MATCH_RMSD, IMAGINARY_CUTOFF

            group = artifact.create_group("transition")
            for key in ("sella_version", "source_job", "seed_image", "steps", "imaginary_count"):
                group.attrs[key] = transition[key]
            group.attrs["rmsd_cutoff_angstrom"] = MATCH_RMSD
            group.attrs["rmsd_separation_angstrom"] = 0.05
            group.attrs["imaginary_cutoff_cm1"] = IMAGINARY_CUTOFF
            group.attrs["ts_fmax_ev_per_angstrom"] = FMAX
            group.attrs["irc_fmax_ev_per_angstrom"] = IRC_FMAX
            group.attrs["irc_dx_angstrom_sqrt_amu"] = 0.08
            group.create_dataset("frequencies", data=transition["frequencies_cm1"]).attrs["unit"] = "cm^-1"
            group.create_dataset("hessian", data=hessian).attrs["unit"] = "hartree/bohr^2"
            group.attrs["branches_json"] = json.dumps(transition["branches"])
            for direction, frames in zip(("forward", "reverse"), trajectories):
                group.create_dataset(direction, data=frames, compression="gzip").attrs["unit"] = "angstrom"
        if band is not None:
            group = artifact.create_group("neb")
            group.attrs["ase_version"] = band.summary["ase_version"]
            group.attrs["climb"] = band.summary["climb"]
            group.attrs["steps"] = band.summary["steps"]
            group.attrs["fmax_ev_per_angstrom"] = 0.05
            group.attrs["spring_ev_per_angstrom_squared"] = 0.1
            group.create_dataset("positions", data=band.summary["positions"]).attrs["unit"] = "angstrom"
            group.create_dataset("forces", data=band.forces).attrs["unit"] = "eV/angstrom"
            group.create_dataset("energies", data=[image["energy_hartree"] for image in band.summary["images"]]).attrs["unit"] = "hartree"
        if solver:
            from .fields import write_fields
            fields = write_fields(artifact, solver[0])
    with temporary.open("rb") as stream:
        os.fsync(stream.fileno())
    os.replace(temporary, output_dir / "result.h5")
    result = {"job_id": request["job_id"], "input_hash": request["input_hash"], "method": request["method"],
              "basis": request["basis"], "energy_hartree": energy, "converged": True,
              "engine_version": engine_version, "elapsed_seconds": time.monotonic() - started}
    if settings is not None:
        result["settings"] = settings
    if optimization is not None:
        result["optimization"] = optimization
    if spin_squared is not None:
        result["spin_squared"] = spin_squared
    if refinement is not None:
        result["refinement"] = refinement
    if thermal is not None:
        result["thermochemistry"] = thermal
    if transition is not None:
        result["transition"] = transition
    if band is not None:
        result["neb"] = band.summary
    if fields:
        result["fields"] = fields
    atomic_json(output_dir / "result.json", result)
    return result


def serve(output_dir: Path) -> int:
    protocol_stdout = sys.stdout
    def emit(event):
        print(json.dumps(event, allow_nan=False), file=protocol_stdout, flush=True)
    request = {}
    try:
        line = sys.stdin.buffer.readline(65537)
        if len(line) > 65536:
            raise ValueError("Request too large")
        request = json.loads(line)
        # Engine/library chatter must not corrupt the control stream.
        with contextlib.redirect_stdout(sys.stderr):
            result = calculate(request, output_dir, emit)
        emit({"kind": "result", "schema_version": 1, "calculation": result})
        return 0
    except Exception as error:
        print(f"{type(error).__name__}: {error}", file=sys.stderr)
        code = "calculation_failed"
        if isinstance(error, screening.ScreeningUnavailable):
            code = "screening_missing"
        elif isinstance(error, ArithmeticError) and str(error) in {"not_converged", "optimization_limit", "screening_not_converged", "neb_limit", "transition_limit", "transition_missing", "solvent_not_converged", "refinement_scope", "refinement_not_converged"}:
            code = str(error)
        emit({"kind": "error", "schema_version": 1, "job_id": request.get("job_id", "") if isinstance(request, dict) else "",
              "code": code})
        return 1
