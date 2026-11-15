import copy
from pathlib import Path
import tempfile
import unittest

from ase import units
import h5py
from pyscf import dft, gto, scf

from reactqm_worker.calculation import calculate, validate_request
from test_calculation import request


class OpenShellTests(unittest.TestCase):
    def test_doublet_hydrogen_ion_uhf_and_uks(self):
        for method, solver_type, xc, grid in (
            ("UHF", scf.UHF, None, None),
            ("UKS", dft.UKS, "PBE", 3),
        ):
            with self.subTest(method=method), tempfile.TemporaryDirectory() as root:
                job = request()
                job["method"] = method
                job["settings"] = {
                    **{key: job[key] for key in ("method", "basis", "max_cycle", "conv_tol")},
                    "xc": xc, "grid_level": grid,
                }
                job["document"].update(charge=1, spin=1, atoms=[
                    {"atomic_number": 1, "position_angstrom": [0, 0, 0]},
                    {"atomic_number": 1, "position_angstrom": [0, 0, 1.05]},
                ])

                def direct(distance):
                    molecule = gto.M(atom=[("H", (0, 0, 0)), ("H", (0, 0, distance))],
                                     basis=job["basis"], charge=1, spin=1, verbose=0)
                    solver = solver_type(molecule)
                    solver.conv_tol = 1e-11
                    if xc:
                        solver.xc = xc
                        solver.grids.level = grid
                    energy = solver.kernel()
                    self.assertTrue(solver.converged)
                    return energy, solver.spin_square()[0]

                reference, spin_squared = direct(1.05)
                step = 1e-4
                gradient = (direct(1.05 + step)[0] - direct(1.05 - step)[0]) * units.Bohr / (2 * step)
                result = calculate(job, Path(root), lambda _: None)
                self.assertAlmostEqual(result["energy_hartree"], reference, places=8)
                self.assertAlmostEqual(result["spin_squared"], spin_squared, places=5)
                self.assertEqual(set(result["fields"]), {"density", "homo_alpha", "lumo_alpha", "lumo_beta"})
                with h5py.File(Path(root) / "result.h5") as artifact:
                    self.assertEqual(artifact.attrs["multiplicity"], 2)
                    self.assertAlmostEqual(artifact.attrs["spin_squared"], spin_squared, places=5)
                    self.assertAlmostEqual(artifact["gradient"][1, 2], gradient, delta=2e-4)
                    if xc:
                        self.assertIn("libxc_version", artifact.attrs)
                for spin in (-1, 0, 2, 11):
                    invalid = copy.deepcopy(job)
                    invalid["document"]["spin"] = spin
                    with self.assertRaises(ValueError):
                        validate_request(invalid)
                restricted = copy.deepcopy(job)
                restricted["method"] = restricted["settings"]["method"] = "RKS" if xc else "RHF"
                with self.assertRaisesRegex(ValueError, "Nonzero spin"):
                    validate_request(restricted)
