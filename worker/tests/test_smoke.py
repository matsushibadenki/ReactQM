import unittest
from reactqm_worker.__main__ import smoke


class InstallationSmokeTest(unittest.TestCase):
    def test_h2_energy_gradient_and_storage(self):
        result = smoke()
        self.assertTrue(result["converged"])
        self.assertAlmostEqual(result["energy_hartree"], -1.1167593074, places=6)


if __name__ == "__main__":
    unittest.main()
