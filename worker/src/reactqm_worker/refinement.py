"""Bounded gas-phase, all-electron post-RHF single points at fixed geometries."""
import math


def evaluate(request, positions):
    from pyscf import cc, gto, mp, scf

    molecule = gto.M(
        atom=[(atom["atomic_number"], position) for atom, position in
              zip(request["document"]["atoms"], positions, strict=True)],
        unit="Angstrom", basis=request["basis"], charge=request["document"]["charge"],
        spin=0, verbose=0, max_memory=512,
    )
    count = molecule.nao_nr()
    if count > 64 or molecule.nelectron < 2 or molecule.nelectron >= 2 * count:
        raise ArithmeticError("refinement_scope")
    reference = scf.RHF(molecule)
    reference.max_cycle = request["max_cycle"]
    reference.conv_tol = request["conv_tol"]
    reference.chkfile = None
    rhf = float(reference.kernel())
    if not reference.converged:
        raise ArithmeticError("not_converged")
    correction = 0.0
    if request["refinement"]["method"] == "mp2":
        solver = mp.MP2(reference)
        solver.kernel(with_t2=False)
    else:
        solver = cc.CCSD(reference)
        solver.max_cycle = 50
        solver.conv_tol = 1e-8
        solver.conv_tol_normt = 1e-6
        solver.kernel()
        if not solver.converged:
            raise ArithmeticError("refinement_not_converged")
        correction = float(solver.ccsd_t())
    energy = float(solver.e_tot) + correction
    if not all(map(math.isfinite, (energy, rhf, correction))):
        raise ValueError("Non-finite correlated energy")
    return energy, rhf, correction, count
