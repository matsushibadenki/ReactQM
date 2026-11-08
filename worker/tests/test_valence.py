import unittest
from rdkit import Chem
from rdkit.Chem import AllChem
from reactqm_worker.valence import analyze


def request(xyz, charge=0, spin=0):
    rows = xyz.strip().splitlines()
    symbols = Chem.GetPeriodicTable()
    atoms = [{"atomic_number": symbols.GetAtomicNumber(row.split()[0]), "position_angstrom": list(map(float,row.split()[1:]))} for row in rows[2:]]
    return {"schema_version": 1, "input_hash": "a"*64, "document": {"schema_version":1, "coordinate_unit":"angstrom", "charge":charge, "spin":spin, "atoms":atoms}}

class ValenceTests(unittest.TestCase):
    def test_hydrogen_water_and_double_bonds(self):
        for xyz, expected in [("2\nH2\nH 0 0 0\nH 0 0 .74",[1,1]), ("3\nwater\nO 0 0 0\nH .7586 0 .5043\nH -.7586 0 .5043",[2,1,1]), ("3\nCO2\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0",[2,4,2])]:
            with self.subTest(xyz=xyz):
                result=analyze(request(xyz));self.assertEqual(result['status'],'inferred')
                self.assertEqual([a['valence'] for a in result['atoms']],expected)
        result=analyze(request("3\nCO2\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0"))
        self.assertEqual(result['atoms'][1]['neighbors'],2)
        self.assertEqual([b['order'] for b in result['bonds']],[2,2])

    def test_known_explicit_graphs_charges_multiple_bonds_aromatic(self):
        for smiles in ['C','N','[NH4+]','[OH-]','C=C','C#C','c1ccccc1','[C-]#[O+]']:
            with self.subTest(smiles=smiles):
                mol=Chem.AddHs(Chem.MolFromSmiles(smiles));self.assertEqual(AllChem.EmbedMolecule(mol,randomSeed=42),0)
                Chem.Kekulize(mol,clearAromaticFlags=True);mol.UpdatePropertyCache()
                result=analyze(request(Chem.MolToXYZBlock(mol),Chem.GetFormalCharge(mol)))
                self.assertEqual(result['status'],'inferred',result)
                self.assertEqual([a['valence'] for a in result['atoms']],[a.GetValence(Chem.ValenceType.EXPLICIT) for a in mol.GetAtoms()])
                self.assertEqual(sum(a['formal_charge'] for a in result['atoms']),Chem.GetFormalCharge(mol))
                self.assertEqual(result['engine_version'],Chem.rdBase.rdkitVersion)

    def test_unsupported_failed_and_invalid_never_get_values(self):
        for req in [request('1\nMg\nMg 0 0 0'),request('2\nO2\nO 0 0 0\nO 0 0 1.21',spin=2), request('2\nfar\nH 0 0 0\nH 0 0 3')]:
            result=analyze(req);self.assertNotEqual(result['status'],'inferred');self.assertEqual(result['atoms'],[])
        bad=request('2\nH2\nH 0 0 0\nH 0 0 .74');bad['document']['atoms'][0]['position_angstrom'][0]=float('nan')
        with self.assertRaises(ValueError):analyze(bad)
