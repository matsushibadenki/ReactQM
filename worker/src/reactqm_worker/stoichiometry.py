"""Gas-phase RRHO standard-state comparison for a balanced reaction."""
from collections import Counter
import hashlib
import json
import math

from .calculation import validate_request
from .thermochemistry import analyze, standard_shift


def read_json(path):
    with path.open("rb") as stream:
        data = stream.read(65537)
    if len(data) > 65536:
        raise ValueError("Oversized input")
    return json.loads(data), hashlib.sha256(data).hexdigest()


def compare(manifest_path):
    manifest, digest = read_json(manifest_path)
    return compare_inputs(manifest, digest, lambda file: read_json(manifest_path.parent / file))


class UnstationarySpecies(ValueError):
    pass


def compare_inputs(manifest, digest, read_input, on_species=lambda _: None):
    from pyscf import __version__, lib
    from pyscf.data import nist

    temperature = manifest["temperature_kelvin"]
    quasi = manifest.get("quasi_rrho", False)
    if not 1 <= temperature <= 2000 or type(quasi) is not bool:
        raise ValueError("Invalid thermochemistry conditions")
    lib.num_threads(1)
    settings = None
    entries = {}

    def load(file):
        nonlocal settings
        path = file
        if path not in entries:
            request, fingerprint = read_input(path)
            validate_request(request)
            if (request["method"] not in ("RHF", "UHF", "RKS", "UKS")
                    or any(request.get(key) for key in ("optimize", "neb", "transition",
                           "thermochemistry", "refinement", "dynamics", "solution", "solution_path"))):
                raise ValueError("Expected a PySCF single-point input")
            condition = tuple(request[key] for key in ("method", "basis", "max_cycle", "conv_tol"))
            if request.get("settings") is not None:
                condition += (request["settings"]["xc"], request["settings"]["grid_level"])
            else:
                condition += (None, None)
            if settings is None:
                settings = condition
            elif settings != condition:
                raise ValueError("Electronic settings must agree across species")
            entries[path] = request, fingerprint
        return entries[path]

    sides = {}
    inventory = {}
    for role in ("reactants", "products"):
        terms = manifest[role]
        if not isinstance(terms, list) or not 1 <= len(terms) <= 12:
            raise ValueError("Each side requires 1–12 terms")
        inventory[role] = Counter()
        sides[role] = []
        for item in terms:
            coefficient = item["coefficient"]
            if type(coefficient) is not int or not 1 <= coefficient <= 16:
                raise ValueError("Coefficients must be positive integers up to 16")
            request, fingerprint = load(item["input"])
            document = request["document"]
            for atom in document["atoms"]:
                inventory[role][atom["atomic_number"]] += coefficient
            inventory[role]["charge"] += coefficient * document["charge"]
            sides[role].append((coefficient, request, fingerprint))
    if inventory["reactants"] != inventory["products"]:
        raise ValueError("Reaction must conserve elements and charge")
    saddle = None
    if "saddle" in manifest:
        request, fingerprint = load(manifest["saddle"])
        catalog = Counter(atom["atomic_number"] for atom in request["document"]["atoms"])
        catalog["charge"] = request["document"]["charge"]
        if catalog != inventory["reactants"]:
            raise ValueError("Saddle must match the balanced composition and charge")
        saddle = (1, request, fingerprint)

    calculated = {}

    def evaluate_term(term, is_saddle=False):
        coefficient, request, fingerprint = term
        key = (fingerprint, is_saddle)
        if key not in calculated:
            on_species(len(calculated) + 1)
            positions = [atom["position_angstrom"] for atom in request["document"]["atoms"]]
            report, _, _ = analyze(request, positions, temperature, is_saddle, {"quasi_rrho": quasi})
            if report["corrections"] is None:
                raise UnstationarySpecies(f"Stationary-point checks failed for {request['document']['name']}")
            calculated[key] = report
        return {"coefficient": coefficient, "name": request["document"]["name"],
                "job_id": request["job_id"], "input_sha256": fingerprint, **calculated[key]}

    results = {role: [evaluate_term(term) for term in terms] for role, terms in sides.items()}
    transition = evaluate_term(saddle, True) if saddle else None
    counts = {role: sum(item["coefficient"] for item in terms) for role, terms in results.items()}
    totals = {role: math.fsum(item["coefficient"] * item["corrections"]["gibbs_hartree"]
                              for item in terms) for role, terms in results.items()}
    factor = nist.HARTREE2J * nist.AVOGADRO / 1000
    shifts = {"reaction": counts["products"] - counts["reactants"],
              "forward": 1 - counts["reactants"], "reverse": 1 - counts["products"]}
    atmosphere = {"reaction": (totals["products"] - totals["reactants"]) * factor}
    if transition is not None:
        gibbs = transition["corrections"]["gibbs_hartree"]
        atmosphere.update(forward=(gibbs - totals["reactants"]) * factor,
                          reverse=(gibbs - totals["products"]) * factor)
    shift = standard_shift(temperature, "molar") * factor
    return {"schema_version": 1, "manifest_sha256": digest, "temperature_kelvin": temperature,
            "quasi_rrho": quasi, "pyscf_version": __version__, "electronic_settings": dict(zip(
                ("method", "basis", "max_cycle", "conv_tol", "xc", "grid_level"), settings, strict=True)),
            **results, "saddle": transition, "species_counts": counts,
            "delta_species_count": {key: shifts[key] for key in atmosphere},
            "per_species_shift_kj_mol": shift, "atmosphere_kj_mol": atmosphere,
            "molar_kj_mol": {key: value + shifts[key] * shift for key, value in atmosphere.items()}}


def compare_job(request, emit):
    """Adapt frozen desktop species to the same evaluator as the CLI."""
    inputs = request["stoichiometry"]
    manifest = {key: inputs[key] for key in ("temperature_kelvin", "quasi_rrho")}
    sources = {}

    def add(term):
        source = {key: value for key, value in request.items() if key != "stoichiometry"}
        source.update(job_id=term["job_id"], document=term["document"], input_hash=term["input_hash"])
        fingerprint = hashlib.sha256(json.dumps(source, sort_keys=True, allow_nan=False).encode()).hexdigest()
        sources[fingerprint] = source, fingerprint
        return fingerprint

    for role in ("reactants", "products"):
        manifest[role] = [{"input": add(term), "coefficient": term["coefficient"]} for term in inputs[role]]
    if inputs["saddle"] is not None:
        manifest["saddle"] = add(inputs["saddle"])

    def progress(cycle):
        emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": cycle})

    digest = hashlib.sha256(json.dumps(inputs, sort_keys=True, allow_nan=False).encode()).hexdigest()
    return compare_inputs(manifest, digest, sources.__getitem__, progress)
