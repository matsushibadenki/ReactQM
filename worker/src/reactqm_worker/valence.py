"""Bounded XYZ bond-order perception, separate from quantum calculations.

The result is a geometry/charge-based hypothesis, never a measured valence.
All hydrogens must be explicit. Closed-shell main-group molecules only.
"""
import json
import math
import sys

SUPPORTED = {1, 5, 6, 7, 8, 9, 14, 15, 16, 17, 35, 53}


def analyze(request):
    from rdkit import Chem, rdBase
    from rdkit.Chem import rdDetermineBonds

    document = request["document"]
    atoms = document["atoms"]
    if request.get("schema_version") != 1 or document.get("schema_version") != 1 or document.get("coordinate_unit") != "angstrom":
        raise ValueError("schema")
    if not isinstance(request.get("input_hash"), str) or len(request["input_hash"]) != 64:
        raise ValueError("hash")
    if not 1 <= len(atoms) <= 64 or type(document["charge"]) is not int or abs(document["charge"]) > 64 or type(document["spin"]) is not int or document["spin"] < 0:
        raise ValueError("input")
    for index, atom in enumerate(atoms):
        xyz = atom["position_angstrom"]
        if type(atom["atomic_number"]) is not int or not 1 <= atom["atomic_number"] <= 118 or len(xyz) != 3 or any(type(v) not in (int, float) or not math.isfinite(v) or abs(v) > 10000 for v in xyz):
            raise ValueError("atom")
        for other in atoms[:index]:
            if sum((a-b)**2 for a,b in zip(xyz, other["position_angstrom"])) < .01:
                raise ValueError("overlap")
    result = {"schema_version": 1, "input_hash": request["input_hash"], "engine_version": rdBase.rdkitVersion,
              "status": "unsupported", "reason": None, "atoms": [], "bonds": []}
    if document["spin"] != 0:
        return result | {"reason": "open_shell"}
    if any(a["atomic_number"] not in SUPPORTED for a in atoms):
        return result | {"reason": "element"}
    electrons = sum(a["atomic_number"] for a in atoms) - document["charge"]
    if electrons <= 0 or electrons % 2:
        raise ValueError("electrons")
    rows = [str(len(atoms)), "ReactQM valence"] + [f'{Chem.GetPeriodicTable().GetElementSymbol(a["atomic_number"])} ' + ' '.join(format(v, '.17g') for v in a["position_angstrom"]) for a in atoms]
    try:
        molecule = Chem.MolFromXYZBlock('\n'.join(rows)+'\n')
        if molecule is None:
            raise ValueError("xyz")
        rdDetermineBonds.DetermineBonds(molecule, charge=document["charge"], allowChargedFragments=True,
                                        embedChiral=False, useVdw=False, maxIterations=1000)
        Chem.SanitizeMol(molecule)
        Chem.Kekulize(molecule, clearAromaticFlags=True)
        molecule.UpdatePropertyCache(strict=True)
        if any(a.GetNumImplicitHs() or a.GetNumRadicalElectrons() for a in molecule.GetAtoms()):
            return result | {"status": "failed", "reason": "incomplete"}
        if sum(a.GetFormalCharge() for a in molecule.GetAtoms()) != document["charge"]:
            raise ValueError("charge")
        perceived = [{"atomic_number": a.GetAtomicNum(), "valence": a.GetValence(Chem.ValenceType.EXPLICIT),
                      "formal_charge": a.GetFormalCharge(), "neighbors": a.GetDegree()} for a in molecule.GetAtoms()]
        bonds = [{"a": b.GetBeginAtomIdx(), "b": b.GetEndAtomIdx(), "order": int(b.GetBondTypeAsDouble())} for b in molecule.GetBonds()]
        return result | {"status": "inferred", "atoms": perceived, "bonds": bonds}
    except Exception:
        return result | {"status": "failed", "reason": "bond_assignment"}


def serve():
    try:
        data = sys.stdin.buffer.read(32769)
        if len(data) > 32768:
            raise ValueError("size")
        print(json.dumps(analyze(json.loads(data)), allow_nan=False), flush=True)
        return 0
    except Exception:
        return 1
