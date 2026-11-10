"""Bounded single-molecule imports; ETKDG initial geometry, not quantum optimization."""
import hashlib
import io
import json
import math
import sys

MAX_TEXT = 16384
SUPPORTED = {1, 5, 6, 7, 8, 9, 14, 15, 16, 17, 35, 53}


def parse_input(format, text):
    from rdkit import Chem
    if format not in ("smiles", "sdf") or not isinstance(text, str) or not text.strip() or len(text.encode("utf-8")) > MAX_TEXT or any(ord(c) < 32 and c not in "\n\r\t" for c in text):
        raise ValueError("structure_input")
    if format == "smiles":
        # No optional name/CXSMILES suffix: do not silently ignore input tokens.
        if any(c.isspace() for c in text.strip()):
            raise ValueError("structure_input")
        params = Chem.SmilesParserParams()
        params.removeHs = False
        params.parseName = False
        params.allowCXSMILES = False
        mol = Chem.MolFromSmiles(text.strip(), params)
    else:
        supplier = Chem.ForwardSDMolSupplier(io.BytesIO(text.encode("utf-8")), sanitize=True, removeHs=False, strictParsing=True)
        records = list(supplier)
        if len(records) != 1:
            raise ValueError("structure_single")
        mol = records[0]
    if mol is None or mol.GetNumAtoms() == 0:
        raise ValueError("structure_input")
    if len(Chem.GetMolFrags(mol)) != 1:
        raise ValueError("structure_single")
    if any(a.HasQuery() or a.GetAtomicNum() not in SUPPORTED or a.GetIsotope() or a.GetNumRadicalElectrons() for a in mol.GetAtoms()):
        raise ValueError("structure_unsupported")
    if any(b.HasQuery() or b.GetBondType() not in (Chem.BondType.SINGLE, Chem.BondType.DOUBLE, Chem.BondType.TRIPLE, Chem.BondType.AROMATIC) for b in mol.GetBonds()):
        raise ValueError("structure_unsupported")
    if mol.GetNumAtoms() > 64:
        raise ValueError("atom_limit")
    mol = Chem.AddHs(mol, addCoords=format == "sdf" and mol.GetNumConformers() > 0 and mol.GetConformer().Is3D())
    if mol.GetNumAtoms() > 64:
        raise ValueError("atom_limit")
    return mol


def generate(request):
    from rdkit import Chem, rdBase
    from rdkit.Chem import AllChem
    if request.get("schema_version") != 1 or type(request.get("seed")) is not int or not 0 <= request["seed"] <= 2147483647:
        raise ValueError("structure_input")
    format, text, seed = request.get("format"), request.get("text"), request["seed"]
    mol = parse_input(format, text)
    charge = Chem.GetFormalCharge(mol)
    electrons = sum(a.GetAtomicNum() for a in mol.GetAtoms()) - charge
    if abs(charge) > 64 or electrons <= 0 or electrons % 2:
        raise ValueError("structure_unsupported")
    canonical = Chem.MolToSmiles(Chem.RemoveHs(mol), isomericSmiles=True)
    method = "sdf_3d" if format == "sdf" and mol.GetNumConformers() and mol.GetConformer().Is3D() else "etkdgv3"
    if method == "etkdgv3":
        mol.RemoveAllConformers()
        params = AllChem.ETKDGv3()
        params.randomSeed = seed
        params.numThreads = 1
        params.maxIterations = 1000
        params.enforceChirality = True
        if AllChem.EmbedMolecule(mol, params) != 0:
            raise ValueError("structure_embed")
    conf = mol.GetConformer()
    atoms = [{"atomic_number": a.GetAtomicNum(), "position_angstrom": list(conf.GetAtomPosition(a.GetIdx()))} for a in mol.GetAtoms()]
    for i, atom in enumerate(atoms):
        xyz = atom["position_angstrom"]
        if any(not math.isfinite(v) or abs(v) > 10000 for v in xyz):
            raise ValueError("coordinate")
        for other in atoms[:i]:
            if sum((a-b)**2 for a,b in zip(xyz, other["position_angstrom"])) < .01:
                raise ValueError("atom_overlap")
    name = mol.GetProp("_Name").strip() if mol.HasProp("_Name") else canonical
    name = name or canonical
    return {"schema_version": 1, "name": name[:120], "coordinate_unit": "angstrom", "atoms": atoms, "charge": charge, "spin": 0,
            "source": {"format": format, "text": text, "input_hash": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                       "canonical_smiles": canonical, "engine_version": rdBase.rdkitVersion, "method": method,
                       "seed": seed if method == "etkdgv3" else None}}


def serve():
    try:
        data = sys.stdin.buffer.read(65537)
        if len(data) > 65536:
            raise ValueError("structure_input")
        result = {"document": generate(json.loads(data)), "error": None}
    except Exception as error:
        code = str(error)
        allowed = {"structure_input", "structure_single", "structure_unsupported", "structure_embed", "atom_limit", "coordinate", "atom_overlap"}
        result = {"document": None, "error": code if code in allowed else "structure_input"}
    print(json.dumps(result, ensure_ascii=False, allow_nan=False), flush=True)
    return 0
