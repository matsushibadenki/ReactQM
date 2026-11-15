import copy
import tempfile
import unittest
from pathlib import Path

from ase import units
from pyscf import gto, scf

from reactqm_worker.optimization import optimize
from test_calculation import request


class OptimizationTests(unittest.TestCase):
    def test_checkpoint_resume_converges_to_independent_h2_minimum(self):
        job = request()
        job["document"]["atoms"][1]["position_angstrom"][2] = 1.1
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root) / "optimization"
            first = optimize(job, directory, 0.01, 1, False)
            self.assertFalse(first["converged"])
            self.assertEqual(first["steps"], 1)
            self.assertFalse((directory / "optimization-result.json").exists())
            changed = copy.deepcopy(job)
            changed["conv_tol"] = 1e-10
            with self.assertRaisesRegex(ValueError, "Checkpoint does not match"):
                optimize(changed, directory, 0.01, 30, True)
            final = optimize(job, directory, 0.01, 30, True)
            self.assertTrue(final["converged"])
            self.assertGreater(final["steps"], 1)
            self.assertLess(final["energy_hartree"], first["energy_hartree"])
            self.assertTrue((directory / "optimization-result.json").exists())

            from scipy.optimize import minimize_scalar
            def energy(distance):
                molecule = gto.M(atom=[("H", (0, 0, 0)), ("H", (0, 0, distance))],
                                 basis="sto-3g", verbose=0)
                return scf.RHF(molecule).kernel()
            reference = minimize_scalar(energy, bounds=(0.6, 0.9), method="bounded")
            self.assertTrue(reference.success)
            a, b = final["positions_angstrom"]
            distance = sum((x-y)**2 for x, y in zip(a, b, strict=True))**0.5
            self.assertAlmostEqual(distance, reference.x, delta=0.002)
            self.assertAlmostEqual(final["energy_hartree"], reference.fun, delta=1e-6)
            step = 1e-4
            force = abs(energy(distance + step) - energy(distance - step)) * units.Hartree / (2 * step)
            self.assertAlmostEqual(final["max_force_ev_per_angstrom"], force, delta=2e-5)
            self.assertLess(force, 0.01)

    def test_pbe_water_lowers_energy_and_meets_force_threshold(self):
        job = request()
        job.update(method="RKS", basis="6-31g")
        job["settings"] = {
            "method": "RKS", "basis": "6-31g", "xc": "PBE", "grid_level": 3,
            "max_cycle": job["max_cycle"], "conv_tol": job["conv_tol"],
        }
        job["document"]["atoms"] = [
            {"atomic_number": 8, "position_angstrom": [0, 0, 0]},
            {"atomic_number": 1, "position_angstrom": [0.9, 0, 0.6]},
            {"atomic_number": 1, "position_angstrom": [-0.9, 0, 0.6]},
        ]
        from reactqm_worker.calculation import evaluate
        initial_energy, _, _, _ = evaluate(job, lambda _: None)
        with tempfile.TemporaryDirectory() as root:
            result = optimize(job, Path(root) / "optimization", 0.05, 40, False)
        self.assertTrue(result["converged"])
        self.assertLess(result["energy_hartree"], initial_energy)
        self.assertLess(result["max_force_ev_per_angstrom"], 0.05)
        self.assertTrue(result["identity"]["libxc_version"])
