//! Scientific data stays in Rust. Coordinates are in angstrom, energy in hartree.
#[derive(Default)]
pub struct Document {
    pub atoms: Vec<Atom>,
    pub charge: i32,
    /// Number of unpaired electrons (2S), not multiplicity (2S + 1).
    pub spin: u32,
}

pub struct Atom {
    pub atomic_number: u8,
    pub position_angstrom: [f64; 3],
}
