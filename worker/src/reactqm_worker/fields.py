"""Sample PySCF orbitals and total electron density on a bounded Cartesian grid."""
import hashlib
import io
import json
import struct
import sys

SIZE = 40
FIELDS = {"density", "homo", "lumo", "homo_alpha", "lumo_alpha", "homo_beta", "lumo_beta"}


def write_fields(artifact, solver):
    import numpy as np
    from ase import units
    from pyscf.dft import numint

    molecule = solver.mol
    atoms = molecule.atom_coords()
    origin = atoms.min(axis=0) - 5.0
    step = (atoms.max(axis=0) + 5.0 - origin) / (SIZE - 1)
    energies, occupations, coefficients = map(np.asarray, (solver.mo_energy, solver.mo_occ, solver.mo_coeff))
    channels = [(energies, occupations, coefficients, "")] if coefficients.ndim == 2 else [
        (energies[i], occupations[i], coefficients[i], suffix)
        for i, suffix in enumerate(("_alpha", "_beta"))
    ]
    frontiers = {}
    for energies, occupations, coefficients, suffix in channels:
        for label, mask, choose in (("homo", occupations > 1e-8, np.argmax),
                                    ("lumo", occupations <= 1e-8, np.argmin)):
            indices = np.flatnonzero(mask)
            if indices.size:
                index = indices[choose(energies[indices])]
                frontiers[label + suffix] = (coefficients[:, index], int(index), float(energies[index]))
    matrix = np.asarray(solver.make_rdm1())
    if matrix.ndim == 3:
        matrix = matrix.sum(axis=0)
    values = {key: np.empty(SIZE**3, dtype=np.float32) for key in ["density", *frontiers]}
    for start in range(0, SIZE**3, 2048):
        end = min(start + 2048, SIZE**3)
        offsets = np.array(np.unravel_index(np.arange(start, end), (SIZE,) * 3)).T
        ao = numint.eval_ao(molecule, origin + offsets * step)
        values["density"][start:end] = numint.eval_rho(molecule, ao, matrix)
        for key, (coefficient, _, _) in frontiers.items():
            values[key][start:end] = ao @ coefficient
    group = artifact.create_group("fields")
    group.attrs["schema_version"] = 1
    group.attrs["origin_angstrom"] = origin * units.Bohr
    group.attrs["step_angstrom"] = step * units.Bohr
    available = []
    for key, samples in values.items():
        if not np.isfinite(samples).all():
            continue
        iso = 0.02 if key == "density" else 0.03
        if np.abs(samples).max() <= iso:
            continue
        field = group.create_dataset(key, data=samples.reshape((SIZE,) * 3), compression="gzip")
        field.attrs["unit"] = "electron/bohr^3" if key == "density" else "bohr^-3/2"
        if key in frontiers:
            _, index, energy = frontiers[key]
            field.attrs["orbital_index"] = index + 1
            field.attrs["orbital_energy_hartree"] = energy
        available.append(key)
    return available


def serve():
    import h5py
    import numpy as np

    try:
        request = json.loads(sys.stdin.buffer.readline(65537))
        field = request["field"]
        if field not in FIELDS:
            return 1
        limit = 4 * 1024 * 1024
        with open(request["artifact"], "rb") as source:
            contents = source.read(limit + 1)
        if len(contents) > limit or hashlib.sha256(contents).hexdigest() != request["sha256"]:
            return 1
        with h5py.File(io.BytesIO(contents), "r") as artifact:
            if artifact.attrs["job_id"] != request["job_id"] or artifact.attrs["input_hash"] != request["input_hash"]:
                return 1
            group = artifact["fields"]
            dataset = group[field]
            if group.attrs["schema_version"] != 1 or dataset.shape != (SIZE,) * 3:
                return 1
            origin, step = (np.asarray(group.attrs[key]) for key in ("origin_angstrom", "step_angstrom"))
            if origin.shape != (3,) or step.shape != (3,):
                return 1
            header = struct.pack("<I6d", SIZE, *origin, *step)
            data = np.asarray(dataset, dtype="<f4").tobytes()
        sys.stdout.buffer.write(header + data)
        return 0
    except (KeyError, ValueError, TypeError, OSError):
        return 1
