from pathlib import Path
import tempfile
import unittest

from ase import units
import h5py
import numpy as np

from reactqm_worker.calculation import calculate
from test_calculation import request


class FieldsTests(unittest.TestCase):
    def test_hydrogen_frontier_signs_and_density_integral(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            result = calculate(request(), path, lambda _: None)
            self.assertEqual(set(result["fields"]), {"density", "homo", "lumo"})
            with h5py.File(path / "result.h5") as artifact:
                group = artifact["fields"]
                density, homo, lumo = (group[key][:] for key in ("density", "homo", "lumo"))
                self.assertTrue(np.all(density >= 0))
                volume_bohr = np.prod(group.attrs["step_angstrom"] / units.Bohr)
                self.assertAlmostEqual(density.sum() * volume_bohr, 2.0, delta=0.04)
                np.testing.assert_allclose(homo, homo[:, :, ::-1], atol=1e-6)
                np.testing.assert_allclose(lumo, -lumo[:, :, ::-1], atol=1e-6)
                self.assertEqual(group["homo"].attrs["orbital_index"], 1)
                self.assertEqual(group["lumo"].attrs["orbital_index"], 2)
                self.assertGreater(lumo.max(), 0.03)
                self.assertLess(lumo.min(), -0.03)
