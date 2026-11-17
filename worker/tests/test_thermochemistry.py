from pathlib import Path
import tempfile
import unittest
import uuid

import h5py
from ase import units

from reactqm_worker.calculation import calculate
from test_calculation import request


class ThermochemistryTests(unittest.TestCase):
    def test_hcn_hnc_stationary_points_and_temperature_dependence(self):
        # Structures optimized with RHF/STO-3G; coordinates are aligned independently.
        reactant = [[0, 0, 0.00230468417], [0, 0, 1.15531220276], [0, 0, -1.06761688693]]
        product = [[0, 0, -0.00055220749], [0, 0, 1.16973363427], [0, 0, 2.18081857322]]
        saddle = [[-0.00847226371, 0, -0.00333052927], [-0.10077008431, 0, 1.21459495222],
                  [1.10924234802, 0, 0.43873557705]]
        job = request()
        job["document"]["atoms"] = [{"atomic_number": z, "position_angstrom": position}
                                    for z, position in zip((6, 7, 1), saddle)]
        job["thermochemistry"] = {"source_job": str(uuid.uuid4()), "temperature_kelvin": 298.15,
                                  "reactant": reactant, "product": product}
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)
            reports = []
            for temperature in (298.15, 500.0):
                job["thermochemistry"]["temperature_kelvin"] = temperature
                result = calculate(job, path, lambda _: None)["thermochemistry"]
                reports.append(result)
                for label in ("reactant", "product", "saddle"):
                    species = result[label]
                    self.assertEqual(sum(value < 0 for value in species["frequencies_cm1"]),
                                     int(label == "saddle"))
                    expected_zpe = sum(max(0, value) for value in species["frequencies_cm1"]) * units.invcm / (2 * units.Hartree)
                    self.assertAlmostEqual(species["corrections"]["zpe_hartree"], expected_zpe, delta=1e-8)
                with h5py.File(path / "result.h5") as artifact:
                    group = artifact["thermochemistry"]
                    self.assertEqual(group.attrs["temperature_kelvin"], temperature)
                    self.assertEqual(group.attrs["pressure_pa"], 101325)
                    self.assertEqual(group["saddle/frequencies"].attrs["unit"], "cm^-1")
            for endpoint, expected_zpe_barrier, expected_gibbs_barriers in (
                ("reactant", 270.6977, [266.1578, 262.4733]),
                ("product", 191.6571, [187.1981, 183.9962]),
            ):
                barriers = [report["saddle"]["corrections"]["gibbs_hartree"] -
                            report[endpoint]["corrections"]["gibbs_hartree"] for report in reports]
                for report, gibbs, expected_gibbs in zip(reports, barriers, expected_gibbs_barriers, strict=True):
                    saddle = report["saddle"]
                    start = report[endpoint]
                    zpe_barrier = (saddle["electronic_hartree"] - start["electronic_hartree"]
                                   + saddle["corrections"]["zpe_hartree"] - start["corrections"]["zpe_hartree"])
                    self.assertAlmostEqual(zpe_barrier * 2625.499639, expected_zpe_barrier, delta=0.02)
                    self.assertAlmostEqual(gibbs * 2625.499639, expected_gibbs, delta=0.02)

    def test_pcm_barriers_and_molar_conversion(self):
        reactant = [[0, 0, .00230468417], [0, 0, 1.15531220276], [0, 0, -1.06761688693]]
        product = [[0, 0, -.00055220749], [0, 0, 1.16973363427], [0, 0, 2.18081857322]]
        saddle = [[-.00847226371, 0, -.00333052927], [-.10077008431, 0, 1.21459495222],
                  [1.10924234802, 0, .43873557705]]
        job = request()
        job["document"]["atoms"] = [{"atomic_number": z, "position_angstrom": position}
                                    for z, position in zip((6, 7, 1), saddle)]
        job["thermochemistry"] = {"source_job": str(uuid.uuid4()), "temperature_kelvin": 298.15,
                                  "reactant": reactant, "product": product}
        with tempfile.TemporaryDirectory() as d:
            path = Path(d)
            gas = calculate(job, path, lambda _: None)["thermochemistry"]
            for model, medium, dielectric, expected in (
                ("c_pcm", "water", 78.3553, (267.39877, 189.43902)),
                ("ief_pcm", "acetonitrile", 35.688, (267.41938, 189.43756)),
            ):
                job["thermochemistry"].update(standard_state="molar", solvent={"model": model, "medium": medium})
                report = calculate(job, path, lambda _: None)["thermochemistry"]
                shift = report["standard_shift_hartree"]
                self.assertAlmostEqual(shift * 2625.499639, 7.92587, places=4)
                for label in ("reactant", "product", "saddle"):
                    self.assertAlmostEqual(report[label]["corrections"]["gibbs_hartree"] -
                                           gas[label]["corrections"]["gibbs_hartree"], shift, places=10)
                    self.assertEqual(report[label]["corrections"]["zpe_hartree"], gas[label]["corrections"]["zpe_hartree"])
                def solvated_gibbs(species):
                    return species["corrections"]["gibbs_hartree"] + species["solvated_electronic_hartree"] - species["electronic_hartree"]
                for endpoint, reference in zip(("reactant", "product"), expected, strict=True):
                    value = (solvated_gibbs(report["saddle"]) - solvated_gibbs(report[endpoint])) * 2625.499639
                    self.assertAlmostEqual(value, reference, delta=0.02)
                with h5py.File(path / "result.h5") as artifact:
                    group = artifact["thermochemistry"]
                    self.assertEqual(group.attrs["pcm_dielectric"], dielectric)
                    self.assertEqual(group.attrs["standard_state"], "molar")
                    self.assertEqual(group["saddle"].attrs["solvated_electronic_hartree"],
                                     report["saddle"]["solvated_electronic_hartree"])
