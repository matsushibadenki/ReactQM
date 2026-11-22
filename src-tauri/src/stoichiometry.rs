use crate::{catalog::Catalog, document::Document, jobs::JobRequest};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub job_id: String,
    pub coefficient: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub job_id: String,
    pub coefficient: u32,
    pub document: Document,
    pub input_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub temperature_kelvin: f64,
    pub quasi_rrho: bool,
    pub reactants: Vec<Term>,
    pub products: Vec<Term>,
    pub saddle: Option<Term>,
}

fn composition(terms: &[Term]) -> (BTreeMap<u8, u32>, i64) {
    let mut elements = BTreeMap::new();
    let mut charge = 0;
    for term in terms {
        for atom in &term.document.atoms {
            *elements.entry(atom.atomic_number).or_default() += term.coefficient;
        }
        charge += i64::from(term.document.charge) * i64::from(term.coefficient);
    }
    (elements, charge)
}

impl Input {
    pub fn validate(&self, request: &JobRequest) -> Result<(), String> {
        if !(1.0..=2000.0).contains(&self.temperature_kelvin)
            || !(1..=8).contains(&self.reactants.len())
            || !(1..=8).contains(&self.products.len())
            || request.document != self.reactants[0].document
            || request.optimize
            || request.neb.is_some()
            || request.transition.is_some()
            || request.thermochemistry.is_some()
            || request.refinement.is_some()
            || request.solution.is_some()
            || request.solution_path.is_some()
            || request.dynamics.is_some()
            || request.method == crate::settings::SCREENING
        {
            return Err("stoichiometry_input".into());
        }
        for term in self
            .reactants
            .iter()
            .chain(&self.products)
            .chain(&self.saddle)
        {
            term.document.validate()?;
            if term.input_hash != term.document.fingerprint() {
                return Err("stoichiometry_input".into());
            }
            if !(1..=16).contains(&term.coefficient)
                || !(1..=16).contains(&term.document.atoms.len())
                || uuid::Uuid::parse_str(&term.job_id).is_err()
                || (term.document.spin != 0 && !request.effective_settings().is_unrestricted())
                || term.document.atoms.iter().any(|a| a.atomic_number > 10)
            {
                return Err("stoichiometry_input".into());
            }
        }
        let reactants = composition(&self.reactants);
        if reactants != composition(&self.products)
            || self.saddle.as_ref().is_some_and(|term| {
                term.coefficient != 1 || composition(std::slice::from_ref(term)) != reactants
            })
        {
            return Err("stoichiometry_balance".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Energy {
    pub reaction: f64,
    pub forward: Option<f64>,
    pub reverse: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SummaryTerm {
    pub coefficient: u32,
    pub name: String,
    pub job_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub temperature_kelvin: f64,
    pub quasi_rrho: bool,
    pub reactants: Vec<SummaryTerm>,
    pub products: Vec<SummaryTerm>,
    pub saddle: Option<String>,
    pub reactant_count: u32,
    pub product_count: u32,
    pub per_species_shift_kj_mol: f64,
    pub atmosphere_kj_mol: Energy,
    pub molar_kj_mol: Energy,
}

impl Report {
    pub fn valid(&self) -> bool {
        let t = self.temperature_kelvin;
        let shift = 0.008314462618 * t * (1000.0 * 8.314462618 * t / 101325.0).ln();
        let r = f64::from(self.reactant_count);
        let p = f64::from(self.product_count);
        let matches = |a: f64, b: f64, n: f64| {
            a.is_finite()
                && b.is_finite()
                && (b - a - n * self.per_species_shift_kj_mol).abs() < 1e-6
        };
        (1.0..=2000.0).contains(&t)
            && (1..=128).contains(&self.reactant_count)
            && (1..=128).contains(&self.product_count)
            && (self.per_species_shift_kj_mol - shift).abs() < 1e-6 * (1.0 + shift.abs())
            && matches(
                self.atmosphere_kj_mol.reaction,
                self.molar_kj_mol.reaction,
                p - r,
            )
            && match (
                self.atmosphere_kj_mol.forward,
                self.atmosphere_kj_mol.reverse,
                self.molar_kj_mol.forward,
                self.molar_kj_mol.reverse,
            ) {
                (None, None, None, None) => true,
                (Some(a), Some(b), Some(c), Some(d)) => {
                    matches(a, c, 1.0 - r)
                        && matches(b, d, 1.0 - p)
                        && (a - b - self.atmosphere_kj_mol.reaction).abs() < 1e-6
                }
                _ => false,
            }
    }

    pub fn matches(&self, input: &Input) -> bool {
        let terms_match = |terms: &[SummaryTerm], source: &[Term]| {
            terms.len() == source.len()
                && terms.iter().zip(source).all(|(a, b)| {
                    a.job_id == b.job_id
                        && a.coefficient == b.coefficient
                        && a.name == b.document.name
                })
        };
        terms_match(&self.reactants, &input.reactants)
            && terms_match(&self.products, &input.products)
            && self.saddle.as_ref() == input.saddle.as_ref().map(|term| &term.job_id)
            && self.valid()
            && self.temperature_kelvin == input.temperature_kelvin
            && self.quasi_rrho == input.quasi_rrho
            && self.reactant_count
                == input
                    .reactants
                    .iter()
                    .map(|term| term.coefficient)
                    .sum::<u32>()
            && self.product_count
                == input
                    .products
                    .iter()
                    .map(|term| term.coefficient)
                    .sum::<u32>()
            && self.atmosphere_kj_mol.forward.is_some() == input.saddle.is_some()
    }
}

pub fn from_history(
    store: &Catalog,
    revision: u64,
    temperature_kelvin: f64,
    quasi_rrho: bool,
    reactants: Vec<Selection>,
    products: Vec<Selection>,
    saddle: Option<String>,
) -> Result<JobRequest, String> {
    if !(1..=8).contains(&reactants.len()) || !(1..=8).contains(&products.len()) {
        return Err("stoichiometry_input".into());
    }
    let mut settings = None;
    let mut load = |selection: Selection, saddle: bool| {
        let project = store.restore(&selection.job_id)?;
        let result = project.calculation.ok_or("stoichiometry_input")?;
        if result.mace.is_some()
            || result.stoichiometry.is_some()
            || result.neb.is_some()
            || result.refinement.is_some()
            || result.solution.is_some()
            || result.solution_path.is_some()
            || result.dynamics.is_some()
            || result.thermochemistry.is_some()
            || (!saddle && result.transition.is_some())
            || settings
                .as_ref()
                .is_some_and(|old| old != &project.settings)
        {
            return Err("stoichiometry_input".into());
        }
        settings.get_or_insert(project.settings);
        let mut document = project.document.ok_or("stoichiometry_input")?;
        document.source = None;
        Ok::<_, String>(Term {
            job_id: selection.job_id,
            coefficient: selection.coefficient,
            input_hash: document.fingerprint(),
            document,
        })
    };
    let reactants = reactants
        .into_iter()
        .map(|term| load(term, false))
        .collect::<Result<Vec<_>, _>>()?;
    let products = products
        .into_iter()
        .map(|term| load(term, false))
        .collect::<Result<Vec<_>, _>>()?;
    let saddle = saddle
        .map(|job_id| {
            load(
                Selection {
                    job_id,
                    coefficient: 1,
                },
                true,
            )
        })
        .transpose()?;
    let mut request = JobRequest::with_settings(
        reactants[0].document.clone(),
        revision,
        settings.ok_or("stoichiometry_input")?,
    )?;
    request.stoichiometry = Some(Input {
        temperature_kelvin,
        quasi_rrho,
        reactants,
        products,
        saddle,
    });
    request.validate()?;
    Ok(request)
}
