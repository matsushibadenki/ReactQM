"""First-order saddle optimization, analytic PySCF frequencies, and Sella IRC."""
from importlib.metadata import version

from .calculation import evaluate

FMAX = 0.02
IRC_FMAX = 0.03
MATCH_RMSD = 0.35
IMAGINARY_CUTOFF = 20.0


def run(request, emit):
    import numpy as np
    from ase import Atoms, units
    from ase.calculators.calculator import Calculator, all_changes
    from pyscf.hessian import thermo
    try:
        from sella import IRC, Sella
        from sella.optimize.irc import IRCInnerLoopConvergenceFailure
    except (ImportError, OSError) as error:
        raise ArithmeticError("transition_missing") from error

    document = request["document"]
    source = request["transition"]
    phase = 0

    def progress():
        emit({"kind": "progress", "schema_version": 1, "job_id": request["job_id"], "cycle": phase})

    def evaluate_image(positions, on_converged=None):
        image = {key: value for key, value in request.items() if key != "transition"}
        image["document"] = {**document, "atoms": [
            {**atom, "position_angstrom": position}
            for atom, position in zip(document["atoms"], positions.tolist(), strict=True)
        ]}
        return evaluate(image, lambda _: None, on_converged=on_converged)

    class Engine(Calculator):
        implemented_properties = ["energy", "forces"]

        def calculate(self, atoms=None, properties=("energy",), system_changes=all_changes):
            super().calculate(atoms, properties, system_changes)
            energy, gradient, _, _ = evaluate_image(self.atoms.positions)
            self.results = {"energy": energy * units.Hartree, "forces": -gradient * units.Hartree / units.Bohr}

    def atoms_at(positions):
        atoms = Atoms(numbers=[atom["atomic_number"] for atom in document["atoms"]], positions=positions)
        atoms.calc = Engine()
        return atoms

    def hessian(solver):
        result = solver.Hessian().kernel()
        if not np.isfinite(result).all():
            raise ValueError("Non-finite Hessian")
        return result

    def analytic_hessian(atoms):
        blocks = []
        evaluate_image(atoms.positions, lambda solver: blocks.append(hessian(solver)))
        return blocks[0].transpose(0, 2, 1, 3).reshape(3 * len(atoms), -1) * units.Hartree / units.Bohr**2

    seed = [atom["position_angstrom"] for atom in document["atoms"]]
    saddle = atoms_at(seed)
    progress()
    with Sella(saddle, order=1, logfile=None, hessian_function=analytic_hessian, diag_every_n=1) as optimizer:
        if not optimizer.run(fmax=FMAX, steps=35):
            raise ArithmeticError("transition_limit")
        steps = optimizer.nsteps
    saddle_force = float(np.linalg.norm(saddle.get_forces(), axis=1).max())
    if saddle_force >= FMAX:
        raise ArithmeticError("transition_limit")
    phase = 1
    progress()
    solvers = []
    energy, gradient, coordinates, spin_squared = evaluate_image(saddle.positions, solvers.append)
    solver = solvers[0]
    force_constants = hessian(solver)
    modes = thermo.harmonic_analysis(solver.mol, force_constants, imaginary_freq=True,
                                    mass=saddle.get_masses())
    frequencies = [float(-abs(value.imag) if value.imag else value.real) for value in modes["freq_wavenumber"]]
    imaginary = sum(value < -IMAGINARY_CUTOFF for value in frequencies)

    def rmsd(reference, actual):
        a = np.asarray(reference) - np.mean(reference, axis=0)
        b = actual - actual.mean(axis=0)
        u, _, vt = np.linalg.svd(a.T @ b)
        u[:, -1] *= np.linalg.det(u @ vt)
        return float(np.sqrt(np.mean(np.sum((a @ u @ vt - b)**2, axis=1))))

    branches, trajectories = [], []
    if imaginary == 1:
        for direction in ("forward", "reverse"):
            phase += 1
            progress()
            atoms = atoms_at(saddle.positions)
            trajectory = []
            with IRC(atoms, dx=0.08, ninner_iter=25, logfile=None) as irc:
                irc.attach(lambda: trajectory.append(atoms.positions.tolist()))
                try:
                    converged = bool(irc.run(fmax=IRC_FMAX, steps=120, direction=direction))
                except IRCInnerLoopConvergenceFailure:
                    converged = False
            if not trajectory or trajectory[-1] != atoms.positions.tolist():
                trajectory.append(atoms.positions.tolist())
            force = float(np.linalg.norm(atoms.get_forces(), axis=1).max())
            converged = converged and force < IRC_FMAX
            a, b = (rmsd(source[key], atoms.positions) for key in ("reactant", "product"))
            match = None
            if converged and min(a, b) <= MATCH_RMSD and abs(a-b) >= 0.05:
                match = "reactant" if a < b else "product"
            branches.append({"converged": converged, "match": match, "reactant_rmsd_angstrom": a,
                             "product_rmsd_angstrom": b, "max_force_ev_per_angstrom": force,
                             "steps": len(trajectory)-1})
            trajectories.append(trajectory)

    result = {"sella_version": version("sella"), "steps": steps, "frequencies_cm1": frequencies,
              "imaginary_count": imaginary, "branches": branches, "positions_angstrom": coordinates.tolist(),
              "max_force_ev_per_angstrom": saddle_force}
    return result, trajectories, force_constants, energy, gradient, coordinates, spin_squared
