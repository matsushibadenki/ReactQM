import copy
import json
import tempfile
import unittest
from pathlib import Path
import h5py
import numpy as np
from pyscf import gto, scf, dft
from test_calculation import request
from reactqm_worker.calculation import calculate, validate_request

class SettingsTests(unittest.TestCase):
    def job(self,method='RKS',xc='PBE',basis='6-31g',grid=3):
        job=request();settings={'method':method,'xc':xc,'basis':basis,'grid_level':grid,'max_cycle':50,'conv_tol':1e-9}
        job.update({k:settings[k] for k in ['method','basis','max_cycle','conv_tol']});job['settings']=settings;return job
    def test_all_methods_and_bases_match_direct_engine_and_preserve_units(self):
        for basis in ['sto-3g','6-31g','def2-svp']:
            for xc in [None,'PBE','PBE0','B3LYP']:
                with self.subTest(basis=basis,xc=xc),tempfile.TemporaryDirectory() as directory:
                    job=self.job('RHF' if xc is None else 'RKS',xc,basis,None if xc is None else 3)
                    result=calculate(job,Path(directory),lambda event:None)
                    reference_mol=gto.M(atom='H 0 0 0;H 0 0 .74',basis=basis,spin=0,verbose=0)
                    reference=scf.RHF(reference_mol) if xc is None else dft.RKS(reference_mol)
                    if xc:reference.xc=xc;reference.grids.level=3
                    reference.conv_tol=1e-9;energy=reference.kernel()
                    self.assertTrue(reference.converged);self.assertAlmostEqual(result['energy_hartree'],energy,places=9)
                    self.assertEqual(result['settings'],job['settings'])
                    with h5py.File(Path(directory)/'result.h5') as artifact:
                        self.assertEqual(json.loads(artifact.attrs['settings_json']),job['settings'])
                        self.assertEqual(artifact['gradient'].attrs['unit'],'hartree/bohr')
                        self.assertTrue(np.isfinite(artifact['gradient'][:]).all())
                        np.testing.assert_allclose(artifact['gradient'][:].sum(axis=0),0,atol=1e-7)
                        if xc:self.assertTrue(artifact.attrs['libxc_version'])
    def test_invalid_and_inconsistent_settings_rejected_before_execution(self):
        job=self.job()
        for field,value in [('xc','invalid'),('basis','cc-pv5z'),('grid_level',True),('grid_level',9),('method','UKS'),('max_cycle',0),('conv_tol',float('nan'))]:
            bad=copy.deepcopy(job);bad['settings'][field]=value
            with self.assertRaises(ValueError):validate_request(bad)
        bad=copy.deepcopy(job);bad['document']['spin']=2
        with self.assertRaises(ValueError):validate_request(bad)
    def test_finer_grid_runs_and_nonconvergence_is_not_accepted(self):
        with tempfile.TemporaryDirectory() as directory:
            result=calculate(self.job(grid=5),Path(directory),lambda event:None)
            self.assertEqual(result['settings']['grid_level'],5)
        job=self.job();job['max_cycle']=job['settings']['max_cycle']=1
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ArithmeticError):calculate(job,Path(directory),lambda event:None)
            self.assertFalse((Path(directory)/'result.h5').exists())
