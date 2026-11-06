import copy
import tempfile
import unittest
import uuid
from pathlib import Path
from reactqm_worker.calculation import calculate, validate_request


def request():
    return {'schema_version': 1, 'job_id': str(uuid.uuid4()), 'document_revision': 1,
            'input_hash': 'a' * 64, 'method': 'RHF', 'basis': 'sto-3g', 'max_cycle': 50, 'conv_tol': 1e-9,
            'document': {'schema_version': 1, 'name': 'H2', 'coordinate_unit': 'angstrom', 'charge': 0, 'spin': 0,
                         'atoms': [{'atomic_number': 1, 'position_angstrom': [0, 0, 0]},
                                   {'atomic_number': 1, 'position_angstrom': [0, 0, 0.74]}]}}


class CalculationTests(unittest.TestCase):
    def test_rejects_inconsistent_requests(self):
        valid = request()
        for field, value in [('coordinate_unit', 'bohr'), ('charge', 1), ('charge', -2**31), ('spin', 2)]:
            invalid = copy.deepcopy(valid)
            invalid['document'][field] = value
            with self.assertRaises(ValueError):
                validate_request(invalid)
        for point in [[float('nan'), 0, 0], [0, 0, 0]]:
            invalid = copy.deepcopy(valid)
            invalid['document']['atoms'][1]['position_angstrom'] = point
            with self.assertRaises(ValueError):
                validate_request(invalid)

    def test_real_result_and_artifact_units(self):
        import h5py
        with tempfile.TemporaryDirectory() as directory:
            events = []
            job = request()
            result = calculate(job, Path(directory), events.append)
            self.assertTrue(result['converged'])
            self.assertAlmostEqual(result['energy_hartree'], -1.1167593074, places=6)
            self.assertTrue(events)
            self.assertEqual(result['input_hash'], job['input_hash'])
            with h5py.File(Path(directory) / 'result.h5') as artifact:
                self.assertEqual(artifact['gradient'].attrs['unit'], 'hartree/bohr')
                self.assertEqual(artifact['coordinates'].attrs['unit'], 'angstrom')
                self.assertEqual(artifact['gradient'].shape, (2, 3))
                self.assertEqual(artifact.attrs['job_id'], job['job_id'])

    def test_nonconvergence_produces_no_success_artifact(self):
        job = request()
        job['max_cycle'] = 1
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ArithmeticError, 'not_converged'):
                calculate(job, Path(directory), lambda event: None)
            self.assertFalse((Path(directory) / 'result.json').exists())
            self.assertFalse((Path(directory) / 'result.h5').exists())
