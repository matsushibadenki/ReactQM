import hashlib
import json
import unittest
from rdkit import Chem
from rdkit.Chem import AllChem
from reactqm_worker.structure import generate
from reactqm_worker.valence import analyze


def request(text, format="smiles", seed=42):
    return {"schema_version": 1, "format": format, "text": text, "seed": seed}


class StructureTests(unittest.TestCase):
    def test_repeatability_charge_stereo_and_graph(self):
        for smiles in ["O", "CCO", "[NH4+]", "[OH-]", "C=C", "C#C", "c1ccccc1", "F[C@H](Cl)Br"]:
            with self.subTest(smiles=smiles):
                a = generate(request(smiles))
                self.assertEqual(a, generate(request(smiles)))
                self.assertEqual(a['source']['input_hash'], hashlib.sha256(smiles.encode()).hexdigest())
                self.assertEqual(a['charge'], Chem.GetFormalCharge(Chem.MolFromSmiles(smiles)))
                self.assertEqual(a['spin'], 0)
                result = analyze({'schema_version':1,'input_hash':'a'*64,'document':a})
                self.assertEqual(result['status'], 'inferred',result)
                self.assertEqual(len(result['atoms']),len(a['atoms']))
        self.assertIn('@',generate(request('F[C@H](Cl)Br'))['source']['canonical_smiles'])
        self.assertNotEqual(generate(request('CCCC',seed=42))['atoms'],generate(request('CCCC',seed=43))['atoms'])

    def test_sdf_2d_generation_and_3d_preservation(self):
        mol = Chem.MolFromSmiles('CCO')
        AllChem.Compute2DCoords(mol)
        sdf2d = Chem.MolToMolBlock(mol)+'\n$$$$\n'
        a = generate(request(sdf2d,'sdf'))
        self.assertEqual(a['source']['method'],'etkdgv3')
        self.assertEqual(a,generate(request(sdf2d,'sdf')))
        mol = Chem.AddHs(mol)
        self.assertEqual(AllChem.EmbedMolecule(mol,randomSeed=12),0)
        sdf3d = Chem.MolToMolBlock(mol)+'\n$$$$\n'
        parsed = Chem.MolFromMolBlock(sdf3d.split('$$$$')[0],removeHs=False)
        b = generate(request(sdf3d,'sdf'))
        self.assertEqual(b['source']['method'],'sdf_3d')
        self.assertIsNone(b['source']['seed'])
        self.assertEqual([list(parsed.GetConformer().GetAtomPosition(i)) for i in range(parsed.GetNumAtoms())], [a['position_angstrom'] for a in b['atoms']])
        self.assertEqual(b,generate(request(sdf3d,'sdf',seed=99)))
        # Missing explicit H in a 3D SDF: retain existing atom coordinates, add H only.
        heavy = Chem.RemoveHs(mol)
        block = Chem.MolToMolBlock(heavy)
        c = generate(request(block,'sdf'))
        self.assertEqual(len(c['atoms']),9)
        self.assertEqual(c['source']['method'],'sdf_3d')
        self.assertEqual(c['atoms'][:3],b['atoms'][:3])
        with self.assertRaisesRegex(ValueError,'structure_single'):
            generate(request(sdf3d+sdf3d,'sdf'))

    def test_rejections_no_silent_truncation_or_spin_guess(self):
        for text in ['', 'C methane', 'C\nO','not smiles','C.C','[Na+]','[13CH4]','[CH3]','C'*64,'C\x00']:
            with self.subTest(text=text):
                with self.assertRaises(ValueError):generate(request(text))
        for seed in [-1,2147483648,True,1.2]:
            with self.assertRaises(ValueError):generate(request('O',seed=seed))
        with self.assertRaises(ValueError):generate(request('X'*16385))
        with self.assertRaises(ValueError):generate(request('bad sdf','sdf'))
        with self.assertRaises(ValueError):generate(request('C |atomProp:0.x.y|'))

    def test_imported_graph_does_not_depend_on_coordinate_perception(self):
        doc=generate(request('C=C'))
        # A translated/distorted drawing must not discard explicit source connectivity.
        for i,atom in enumerate(doc['atoms']):atom['position_angstrom']=[i*5.0,0.0,0.0]
        result=analyze({'schema_version':1,'input_hash':'a'*64,'document':doc})
        self.assertEqual(result['status'],'inferred')
        self.assertTrue(any(b['order']==2 for b in result['bonds']))
