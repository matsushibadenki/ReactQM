//! Validated endpoint snapshots and an explicit reactant-to-product permutation.
use crate::{catalog::CatalogState, document::Document, project::atomic_json, state::AppState};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub struct ReactionService(Mutex<()>);

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    Reactant,
    Product,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reaction {
    revision: u64,
    reactant: Option<Document>,
    product: Option<Document>,
    mapping: Option<Vec<usize>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionView {
    revision: u64,
    reactant: Option<EndpointView>,
    product: Option<EndpointView>,
    mapping: Option<Vec<usize>>,
    mismatch: Option<String>,
}

#[derive(Clone, Serialize)]
struct EndpointView {
    name: String,
    formula: String,
    elements: Vec<u8>,
    charge: i32,
    multiplicity: u32,
}

impl From<&Document> for EndpointView {
    fn from(doc: &Document) -> Self {
        Self {
            name: doc.name.clone(),
            formula: doc.formula(),
            elements: elements(doc),
            charge: doc.charge,
            multiplicity: doc.spin + 1,
        }
    }
}

fn elements(doc: &Document) -> Vec<u8> {
    doc.atoms.iter().map(|atom| atom.atomic_number).collect()
}

impl Reaction {
    fn endpoints(&self) -> Result<(&Document, &Document), &'static str> {
        let (Some(reactant), Some(product)) = (&self.reactant, &self.product) else {
            return Err("reaction_endpoints");
        };
        if reactant.atoms.len() != product.atoms.len() {
            return Err("reaction_count");
        }
        if reactant.charge != product.charge {
            return Err("reaction_charge");
        }
        if reactant.spin != product.spin {
            return Err("reaction_spin");
        }
        let mut a = elements(reactant);
        let mut b = elements(product);
        a.sort_unstable();
        b.sort_unstable();
        if a != b {
            return Err("reaction_elements");
        }
        Ok((reactant, product))
    }
    fn validate_mapping(&self, mapping: &[usize]) -> Result<(), &'static str> {
        let (reactant, product) = self.endpoints()?;
        if reactant.atoms.len() != mapping.len() {
            return Err("reaction_mapping");
        }
        let mut visited = vec![false; mapping.len()];
        for (atom, &target) in reactant.atoms.iter().zip(mapping) {
            let i = target
                .checked_sub(1)
                .filter(|&i| i < mapping.len())
                .ok_or("reaction_mapping")?;
            if atom.atomic_number != product.atoms[i].atomic_number
                || std::mem::replace(&mut visited[i], true)
            {
                return Err("reaction_mapping");
            }
        }
        Ok(())
    }
    fn view(&self) -> ReactionView {
        ReactionView {
            revision: self.revision,
            reactant: self.reactant.as_ref().map(EndpointView::from),
            product: self.product.as_ref().map(EndpointView::from),
            mapping: self.mapping.clone(),
            mismatch: self.endpoints().err().map(str::to_owned),
        }
    }
    fn load(path: &Path) -> Result<Self, String> {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(_) => return Err("storage".into()),
        };
        let mut bytes = Vec::new();
        file.take(524289)
            .read_to_end(&mut bytes)
            .map_err(|_| "storage")?;
        if bytes.len() > 524288 {
            return Err("reaction_invalid".into());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| "reaction_invalid")?;
        for document in [&value.reactant, &value.product].into_iter().flatten() {
            document.validate()?;
        }
        if let Some(mapping) = &value.mapping {
            value.validate_mapping(mapping)?;
        }
        Ok(value)
    }
}

fn operation(
    app: &AppHandle,
    revision: Option<u64>,
    edit: impl FnOnce(&mut Reaction) -> Result<(), String>,
) -> Result<ReactionView, String> {
    let service = app.state::<ReactionService>();
    let _guard = service.0.lock().map_err(|_| "storage")?;
    app.state::<CatalogState>().get()?;
    let path = crate::storage_directory(app)?.join("reaction-v1.json");
    let mut reaction = Reaction::load(&path)?;
    if let Some(revision) = revision {
        if revision != reaction.revision {
            return Err("reaction_stale".into());
        }
        edit(&mut reaction)?;
        reaction.revision = reaction.revision.checked_add(1).ok_or("reaction_invalid")?;
        atomic_json(&path, &reaction)?;
        app.emit("reaction-changed", reaction.view())
            .map_err(|_| "storage")?;
    }
    Ok(reaction.view())
}

#[tauri::command]
pub fn reaction_snapshot(app: AppHandle) -> Result<ReactionView, String> {
    operation(&app, None, |_| Ok(()))
}

#[tauri::command]
pub fn capture_endpoint(
    app: AppHandle,
    endpoint: Endpoint,
    expected_revision: u64,
    document_revision: u64,
) -> Result<ReactionView, String> {
    operation(&app, Some(expected_revision), |reaction| {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|_| "storage")?;
        crate::check_revision(&workspace, document_revision)?;
        let doc = workspace.project.document.clone().ok_or("no_document")?;
        doc.validate()?;
        let slot = match endpoint {
            Endpoint::Reactant => &mut reaction.reactant,
            Endpoint::Product => &mut reaction.product,
        };
        if slot.as_ref() != Some(&doc) {
            *slot = Some(doc);
            reaction.mapping = None;
        }
        Ok(())
    })
}

#[tauri::command]
pub fn save_atom_mapping(
    app: AppHandle,
    expected_revision: u64,
    mapping: Vec<usize>,
) -> Result<ReactionView, String> {
    operation(&app, Some(expected_revision), |reaction| {
        reaction.validate_mapping(&mapping)?;
        reaction.mapping = Some(mapping);
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_permutation_and_roundtrips_reordered_endpoints() {
        let mut reaction = Reaction {
            reactant: Some(
                Document::from_xyz("3\nWater\nO 0 0 0\nH 1 0 0\nH 0 1 0", 0, 0).unwrap(),
            ),
            product: Some(Document::from_xyz("3\nWater\nH 0 1 0\nO 0 0 0\nH 1 0 0", 0, 0).unwrap()),
            ..Default::default()
        };
        for invalid in [[1, 2, 3], [2, 1, 1], [2, 0, 3], [2, 1, 4]] {
            assert_eq!(reaction.validate_mapping(&invalid), Err("reaction_mapping"));
        }
        let mapping = vec![2, 3, 1];
        reaction.validate_mapping(&mapping).unwrap();
        reaction.mapping = Some(mapping.clone());
        let path =
            std::env::temp_dir().join(format!("reactqm-reaction-{}.json", uuid::Uuid::new_v4()));
        atomic_json(&path, &reaction).unwrap();
        let mut loaded = Reaction::load(&path).unwrap();
        assert_eq!(loaded.mapping, Some(mapping));
        std::fs::remove_file(path).unwrap();
        let product = loaded.product.as_mut().unwrap();
        product.charge = 2;
        assert_eq!(loaded.endpoints().unwrap_err(), "reaction_charge");
        let product = loaded.product.as_mut().unwrap();
        product.charge = 0;
        product.spin = 2;
        assert_eq!(loaded.endpoints().unwrap_err(), "reaction_spin");
        let product = loaded.product.as_mut().unwrap();
        product.spin = 0;
        product.atoms[1].atomic_number = 6;
        assert_eq!(loaded.endpoints().unwrap_err(), "reaction_elements");
        loaded.product.as_mut().unwrap().atoms.pop();
        assert_eq!(loaded.endpoints().unwrap_err(), "reaction_count");
    }
}

#[tauri::command]
pub fn start_neb(
    app: AppHandle,
    expected_revision: u64,
    document_revision: u64,
    climb: bool,
) -> Result<ReactionView, String> {
    let service = app.state::<ReactionService>();
    let _guard = service.0.lock().map_err(|_| "storage")?;
    let runner = app.state::<crate::scheduler::Scheduler>();
    let gate = runner.gate.lock().map_err(|_| "storage")?;
    if gate.stopping {
        return Err("queue_stopping".into());
    }
    let store = app.state::<CatalogState>();
    let store = store.get()?;
    let reaction = Reaction::load(&crate::storage_directory(&app)?.join("reaction-v1.json"))?;
    if reaction.revision != expected_revision {
        return Err("reaction_stale".into());
    }
    let mapping = reaction.mapping.as_ref().ok_or("reaction_mapping")?;
    reaction.validate_mapping(mapping)?;
    let (reactant, product) = reaction.endpoints()?;
    let state = app.state::<AppState>();
    let workspace = state.0.lock().map_err(|_| "storage")?;
    crate::check_revision(&workspace, document_revision)?;
    if workspace.project.settings.method == crate::settings::SCREENING {
        return Err("neb_scope".into());
    }
    let mut request = crate::jobs::JobRequest::with_settings(
        reactant.clone(),
        document_revision,
        workspace.project.settings.clone(),
    )?;
    request.neb = Some(crate::neb::NebInput {
        climb,
        product: mapping
            .iter()
            .map(|&index| product.atoms[index - 1].position_angstrom)
            .collect(),
        restart: None,
    });
    store.enqueue(&request, None)?;
    drop(workspace);
    drop(gate);
    runner.wake();
    let _ = app.emit("job-history-changed", ());
    Ok(reaction.view())
}

#[tauri::command]
pub fn show_neb_image(
    app: AppHandle,
    expected_revision: u64,
    job_id: String,
    index: Option<usize>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut workspace = state.0.lock().map_err(|_| "storage")?;
    crate::check_revision(&workspace, expected_revision)?;
    let result = workspace
        .project
        .calculation
        .as_ref()
        .filter(|result| result.job_id == job_id)
        .ok_or("result_invalid")?;
    let path = result.neb.as_ref().ok_or("result_invalid")?;
    let base = workspace.project.document.as_ref().ok_or("no_document")?;
    let index = index.ok_or("result_invalid")?;
    let image = crate::neb::frame(base, path.positions.get(index).ok_or("result_invalid")?)?;
    let center = crate::renderer::scene::Scene::from_document(Some(base)).center;
    let radius = path
        .positions
        .iter()
        .flatten()
        .map(|position| glam::Vec3::from_array(position.map(|v| v as f32)).distance(center) + 0.8)
        .fold(0.5, f32::max);
    workspace
        .renderer
        .preview_path_image(&image, expected_revision, center, radius);
    workspace.neb_image = Some((expected_revision, job_id, image));
    Ok(())
}
