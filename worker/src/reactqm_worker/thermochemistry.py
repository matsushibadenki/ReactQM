"""Gas-phase RRHO thermochemistry at fixed, validated stationary geometries."""
import math

from .calculation import evaluate

PRESSURE = 101325.0
FMAX = 0.03
MODE_CUTOFF = 20.0
SOLVENTS = {"water": 78.3553, "acetonitrile": 35.688}
MODELS = {"c_pcm": "C-PCM", "ief_pcm": "IEF-PCM"}


PCM_GRID = 29
PCM_SCALE = 1.2
PCM_PROBE = 0.0


def pcm_solver(solver, solvent):
    pcm = solver.PCM()
    pcm.with_solvent.method = MODELS[solvent["model"]]
    pcm.with_solvent.eps = SOLVENTS[solvent["medium"]]
    pcm.with_solvent.lebedev_order = PCM_GRID
    pcm.with_solvent.vdw_scale = PCM_SCALE
    pcm.with_solvent.r_probe = PCM_PROBE
    return pcm


def standard_shift(temperature, standard_state):
    from pyscf.data import nist

    if standard_state == "molar":
        return nist.BOLTZMANN * temperature / nist.HARTREE2J * math.log(
            1000 * nist.BOLTZMANN * nist.AVOGADRO * temperature / PRESSURE
        )
    return 0.0


def analyze(request, positions, temperature, saddle, conditions=None):
    import numpy as np
    from ase import units
    from pyscf.hessian import thermo

    conditions = conditions or {}
    document = request["document"]
    image = {key: value for key, value in request.items() if key != "thermochemistry"}
    image["document"] = {**document, "atoms": [
        {**atom, "position_angstrom": position}
        for atom, position in zip(document["atoms"], positions, strict=True)
    ]}
    solvers = []
    energy, gradient, _, spin_squared = evaluate(image, lambda _: None, on_converged=solvers.append)
    solver = solvers[0]
    hessian = solver.Hessian().kernel()
    modes = thermo.harmonic_analysis(solver.mol, hessian, imaginary_freq=True)
    freq = modes["freq_au"]
    signed = [float(-abs(value.imag) if value.imag else value.real) for value in modes["freq_wavenumber"]]
    force = float(np.linalg.norm(gradient, axis=1).max() * units.Hartree / units.Bohr)
    stationary = force < FMAX and sum(value < -MODE_CUTOFF for value in signed) == int(saddle)
    # A soft or near-zero mode makes fixed RRHO unreliable. Preserve diagnostics.
    valid = stationary and all(abs(value) > MODE_CUTOFF for value in signed)
    corrections = None
    if valid:
        thermal = thermo.thermo(solver, freq[freq.real > 0].real, temperature=temperature, pressure=PRESSURE)
        zpe, gibbs = (float(thermal[key][0]) for key in ("ZPE", "G_tot"))
        if math.isfinite(zpe) and math.isfinite(gibbs):
            corrections = {"zpe_hartree": zpe, "gibbs_hartree": gibbs + standard_shift(temperature, conditions.get("standard_state"))}
    result = {
        "electronic_hartree": energy, "frequencies_cm1": signed,
        "max_force_ev_per_angstrom": force, "corrections": corrections,
    }
    if solvent := conditions.get("solvent"):
        density = solver.make_rdm1()
        pcm = pcm_solver(solver, solvent)
        solvated = float(pcm.kernel(dm0=density))
        if not pcm.converged:
            raise ArithmeticError("solvent_not_converged")
        if not math.isfinite(solvated):
            raise ValueError("Nonfinite PCM energy")
        result["solvated_electronic_hartree"] = solvated
    return result, gradient, spin_squared
