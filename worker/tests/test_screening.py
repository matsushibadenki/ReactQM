import copy
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import h5py
import numpy as np
from ase import units

from reactqm_worker.calculation import calculate, validate_request
from reactqm_worker.screening import evaluate
from test_calculation import request


def screening_request():
    job = request()
    job.update(method="GFN2-xTB", basis="gfn2", max_cycle=250, conv_tol=0)
    job["settings"] = {key: job[key] for key in ("method", "basis", "max_cycle", "conv_tol")}
    job["settings"].update(xc=None, grid_level=None)
    job["document"]["atoms"][1] = {"atomic_number": 17, "position_angstrom": [0, 0, 1.27]}
    return job


class ScreeningTests(unittest.TestCase):
    def test_rejects_incompatible_inputs(self):
        for field, value in [("spin", 2), ("charge", 1)]:
            job = screening_request()
            job["document"][field] = value
            with self.assertRaises(ValueError):
                validate_request(job)
        job = screening_request()
        job["document"]["atoms"][1]["atomic_number"] = 87
        with self.assertRaises(ValueError):
            validate_request(job)
        job = screening_request()
        job["optimize"] = True
        with self.assertRaises(ValueError):
            validate_request(job)
        job = screening_request()
        job["max_cycle"] = job["settings"]["max_cycle"] = 50
        with self.assertRaises(ValueError):
            validate_request(job)

    def test_real_hcl_energy_gradient_and_charge(self):
        try:
            from tblite.interface import Calculator
        except ImportError:
            self.skipTest("Install the screening extra to run the real engine test")
        job = screening_request()
        reference = Calculator("GFN2-xTB", [1, 17], np.array([[0, 0, 0], [0, 0, 1.27 / units.Bohr]]),
                               charge=0.0, uhf=0)
        reference.set("verbosity", 0)
        reference.set("temperature", 300 * units.kB / units.Hartree)
        energy = reference.singlepoint().get("energy")
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            result = calculate(job, path, lambda _: None)
            self.assertTrue(result["converged"])
            self.assertAlmostEqual(result["energy_hartree"], energy, places=10)
            self.assertAlmostEqual(energy, -5.052792620592, places=7)
            step = 1e-4
            energies = []
            for delta in (-step, step):
                document = copy.deepcopy(job["document"])
                document["atoms"][1]["position_angstrom"][2] += delta
                energies.append(evaluate(document)[0])
            derivative = (energies[1] - energies[0]) * units.Bohr / (2 * step)
            with h5py.File(path / "result.h5") as artifact:
                self.assertEqual(artifact.attrs["tblite_version"], result["engine_version"])
                self.assertEqual(artifact.attrs["charge"], 0)
                self.assertEqual(artifact.attrs["scc_max_iterations"], 250)
                self.assertEqual(artifact["gradient"].attrs["unit"], "hartree/bohr")
                self.assertEqual(artifact["coordinates"].attrs["unit"], "angstrom")
                self.assertAlmostEqual(artifact["gradient"][1, 2], derivative, delta=1e-5)
            charged = {**job["document"], "charge": 1,
                       "atoms": [{"atomic_number": 11, "position_angstrom": [0, 0, 0]}]}
            self.assertAlmostEqual(evaluate(charged)[0], 0.1954855667, places=7)

    def test_real_scc_failure_does_not_publish_artifacts(self):
        try:
            import tblite.interface
        except ImportError:
            self.skipTest("Install the screening extra to run the real engine test")
        job = screening_request()
        job["max_cycle"] = job["settings"]["max_cycle"] = 1
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            with patch("reactqm_worker.screening.MAX_ITERATIONS", 1):
                with self.assertRaisesRegex(ArithmeticError, "screening_not_converged"):
                    calculate(job, path, lambda _: None)
            self.assertFalse((path / "result.json").exists())
            self.assertFalse((path / "result.h5").exists())
