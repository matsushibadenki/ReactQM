"""Gas-phase GFN2-xTB single points using the independent tblite engine."""
from importlib.metadata import version

METHOD = "GFN2-xTB"
ACCURACY = 1.0
TEMPERATURE_K = 300.0
MAX_ITERATIONS = 250


class ScreeningUnavailable(RuntimeError):
    pass


def evaluate(document):
    import numpy as np
    from ase import units

    try:
        from tblite.exceptions import TBLiteRuntimeError
        from tblite.interface import Calculator
        from tblite.library import get_version
    except (ImportError, OSError) as error:
        raise ScreeningUnavailable("screening_missing") from error
    numbers = np.array([atom["atomic_number"] for atom in document["atoms"]])
    coordinates = np.array([atom["position_angstrom"] for atom in document["atoms"]], dtype=float)
    calculator = Calculator(METHOD, numbers, coordinates / units.Bohr,
                            charge=float(document["charge"]), uhf=document["spin"])
    calculator.set("verbosity", 0)
    calculator.set("accuracy", ACCURACY)
    calculator.set("temperature", TEMPERATURE_K * units.kB / units.Hartree)
    calculator.set("max-iter", MAX_ITERATIONS)
    try:
        result = calculator.singlepoint()
    except TBLiteRuntimeError as error:
        if "SCF not converged" in str(error):
            raise ArithmeticError("screening_not_converged") from error
        raise
    energy = float(result.get("energy"))
    gradient = result.get("gradient")
    if not np.isfinite(energy) or gradient.shape != coordinates.shape or not np.isfinite(gradient).all():
        raise ValueError("Invalid GFN2-xTB energy or gradient")
    engine_version = ".".join(map(str, get_version()))
    return energy, gradient, coordinates, engine_version, version("tblite")
