//! Versioned scientific input. XYZ uses angstrom; spin means 2S.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const MAX_ATOMS: usize = 64;
pub const MAX_XYZ_BYTES: usize = 16_384;
const SYMBOLS: &str = "H He Li Be B C N O F Ne Na Mg Al Si P S Cl Ar K Ca Sc Ti V Cr Mn Fe Co Ni Cu Zn Ga Ge As Se Br Kr Rb Sr Y Zr Nb Mo Tc Ru Rh Pd Ag Cd In Sn Sb Te I Xe Cs Ba La Ce Pr Nd Pm Sm Eu Gd Tb Dy Ho Er Tm Yb Lu Hf Ta W Re Os Ir Pt Au Hg Tl Pb Bi Po At Rn Fr Ra Ac Th Pa U Np Pu Am Cm Bk Cf Es Fm Md No Lr Rf Db Sg Bh Hs Mt Ds Rg Cn Nh Fl Mc Lv Ts Og";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub name: String,
    pub coordinate_unit: String,
    pub atoms: Vec<Atom>,
    pub charge: i32,
    pub spin: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Atom {
    pub atomic_number: u8,
    pub position_angstrom: [f64; 3],
}

impl Document {
    pub fn from_xyz(text: &str, charge: i32, spin: u32) -> Result<Self, String> {
        if text.len() > MAX_XYZ_BYTES {
            return Err("xyz_size".into());
        }
        let mut lines = text.lines();
        let count: usize = lines
            .next()
            .ok_or("xyz_header")?
            .trim()
            .parse()
            .map_err(|_| "xyz_header")?;
        if count == 0 || count > MAX_ATOMS {
            return Err("atom_limit".into());
        }
        let name = lines
            .next()
            .ok_or("xyz_header")?
            .trim()
            .chars()
            .take(120)
            .collect();
        let mut atoms = Vec::with_capacity(count);
        for _ in 0..count {
            let fields: Vec<_> = lines
                .next()
                .ok_or("xyz_count")?
                .split_whitespace()
                .collect();
            if fields.len() != 4 {
                return Err("xyz_row".into());
            }
            let atomic_number = SYMBOLS
                .split_whitespace()
                .position(|s| s == fields[0])
                .ok_or("element")? as u8
                + 1;
            let mut position_angstrom = [0.0; 3];
            for i in 0..3 {
                position_angstrom[i] = fields[i + 1].parse().map_err(|_| "coordinate")?;
            }
            atoms.push(Atom {
                atomic_number,
                position_angstrom,
            });
        }
        if lines.any(|line| !line.trim().is_empty()) {
            return Err("xyz_count".into());
        }
        let document = Self {
            schema_version: 1,
            name,
            coordinate_unit: "angstrom".into(),
            atoms,
            charge,
            spin,
        };
        document.validate()?;
        Ok(document)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.charge.unsigned_abs() > 64 {
            return Err("electron_spin".into());
        }
        if self.schema_version != 1 || self.coordinate_unit != "angstrom" {
            return Err("document_version".into());
        }
        if self.name.chars().count() > 120 {
            return Err("document_name".into());
        }
        if self.atoms.is_empty() || self.atoms.len() > MAX_ATOMS {
            return Err("atom_limit".into());
        }
        let mut electrons: i64 = -i64::from(self.charge);
        for (i, atom) in self.atoms.iter().enumerate() {
            if !(1..=118).contains(&atom.atomic_number) {
                return Err("element".into());
            }
            if atom
                .position_angstrom
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 10_000.0)
            {
                return Err("coordinate".into());
            }
            electrons += i64::from(atom.atomic_number);
            for other in &self.atoms[..i] {
                let distance_squared: f64 = atom
                    .position_angstrom
                    .iter()
                    .zip(other.position_angstrom)
                    .map(|(a, b)| (a - b).powi(2))
                    .sum();
                if distance_squared < 0.01 {
                    return Err("atom_overlap".into());
                }
            }
        }
        let spin = i64::from(self.spin);
        if electrons <= 0 || spin > electrons || (electrons - spin) % 2 != 0 {
            return Err("electron_spin".into());
        }
        Ok(())
    }

    pub fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("validated finite document"))
        )
    }

    pub fn formula(&self) -> String {
        let symbols: Vec<_> = SYMBOLS.split_whitespace().collect();
        let mut counts = BTreeMap::new();
        for atom in &self.atoms {
            *counts
                .entry(symbols[usize::from(atom.atomic_number) - 1])
                .or_insert(0usize) += 1;
        }
        let mut result = String::new();
        let mut append = |symbol: &str, count: usize| {
            result.push_str(symbol);
            if count > 1 {
                result.push_str(&count.to_string());
            }
        };
        if let Some(count) = counts.remove("C") {
            append("C", count);
            if let Some(count) = counts.remove("H") {
                append("H", count);
            }
        }
        for (symbol, count) in counts {
            append(symbol, count);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H2: &str = "2\nHydrogen\nH 0 0 0\nH 0 0 0.74\n";
    #[test]
    fn reads_crlf_and_blank_comment() {
        let doc = Document::from_xyz("2\r\n\r\nH 0 0 0\r\nH 0 0 7.4e-1\r\n", 0, 0).unwrap();
        assert_eq!(doc.formula(), "H2");
        assert_eq!(doc.atoms[1].position_angstrom[2], 0.74);
    }
    #[test]
    fn rejects_invalid_scientific_input() {
        for text in [
            "0\nempty\n",
            "2\nH2\nH 0 0 0\n",
            "1\nH\nX 0 0 0",
            "1\nH\nH NaN 0 0",
            "2\nH2\nH 0 0 0\nH 0 0 0",
            "2\nH2\nH 0 0 0\nH 0 0 0.74\nH 0 0 1",
        ] {
            assert!(Document::from_xyz(text, 0, 0).is_err(), "{text}");
        }
        assert!(Document::from_xyz(H2, 0, 1).is_err());
        assert!(Document::from_xyz(H2, i32::MAX, 0).is_err());
        assert!(Document::from_xyz(H2, i32::MIN, u32::MAX).is_err());
        assert!(Document::from_xyz("1\nH\nH 0 0 0", 0, 1).is_ok());
    }
    #[test]
    fn schema_and_units_are_enforced() {
        let mut doc = Document::from_xyz(H2, 0, 0).unwrap();
        doc.coordinate_unit = "bohr".into();
        assert!(doc.validate().is_err());
        doc.coordinate_unit = "angstrom".into();
        doc.schema_version = 2;
        assert!(doc.validate().is_err());
    }
}
