//! Atomic edits to the shared molecular document.
use crate::{
    check_revision,
    document::{Atom, Bond, Document},
    project::Project,
    publish,
    state::{AppState, Workspace, WorkspaceSnapshot},
};
use serde::Deserialize;
use tauri::{AppHandle, State};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Edit {
    Add {
        symbol: String,
        position: [f64; 3],
    },
    Move {
        atom: usize,
        position: [f64; 3],
    },
    Drag {
        aspect: f32,
        start: [f32; 2],
        end: [f32; 2],
    },
    Delete {
        atom: usize,
    },
    Bond {
        first: usize,
        second: usize,
        order: u8,
    },
}

fn index(number: usize, count: usize) -> Result<usize, String> {
    number
        .checked_sub(1)
        .filter(|&index| index < count)
        .ok_or_else(|| "atom_edit".into())
}

fn apply(workspace: &mut Workspace, revision: u64, edit: Edit) -> Result<(), String> {
    check_revision(workspace, revision)?;
    let mut project = workspace.project.clone();
    if project.document.is_none() {
        let Edit::Add { symbol, position } = edit else {
            return Err("no_document".into());
        };
        let number = Document::atomic_number(&symbol)?;
        let document = Document {
            schema_version: 1,
            name: String::new(),
            coordinate_unit: "angstrom".into(),
            atoms: vec![Atom {
                atomic_number: number,
                position_angstrom: position,
            }],
            charge: 0,
            spin: u32::from(number % 2),
            source: None,
            bonds: None,
        };
        return commit(workspace, project, document);
    }
    let mut document = project.document.take().unwrap();
    let count = document.atoms.len();
    let composition_changed = matches!(edit, Edit::Add { .. } | Edit::Delete { .. });
    match edit {
        Edit::Add { symbol, position } => document.atoms.push(Atom {
            atomic_number: Document::atomic_number(&symbol)?,
            position_angstrom: position,
        }),
        Edit::Move { atom, position } => {
            let atom = &mut document.atoms[index(atom, count)?];
            if atom.position_angstrom == position {
                return Ok(());
            }
            atom.position_angstrom = position;
        }
        Edit::Drag { aspect, start, end } => {
            let (atom, delta) = workspace
                .renderer
                .editor_drag(revision, aspect, start, end)?;
            if delta == [0.0; 3] {
                return Ok(());
            }
            for (coordinate, delta) in document.atoms[atom].position_angstrom.iter_mut().zip(delta)
            {
                *coordinate += delta;
            }
        }
        Edit::Delete { atom } => {
            let atom = index(atom, count)?;
            document.atoms.remove(atom);
            if document.atoms.is_empty() {
                project.calculation = None;
                return workspace.replace_project(project);
            }
            if let Some(bonds) = &mut document.bonds {
                bonds.retain(|bond| !bond.atoms.contains(&atom));
                for endpoint in bonds.iter_mut().flat_map(|bond| &mut bond.atoms) {
                    *endpoint -= usize::from(*endpoint > atom);
                }
            }
        }
        Edit::Bond {
            first,
            second,
            order,
        } => {
            let mut pair = [index(first, count)?, index(second, count)?];
            pair.sort_unstable();
            if pair[0] == pair[1] || order > 3 {
                return Err("bond_edit".into());
            }
            let mut bonds = document
                .bonds
                .take()
                .unwrap_or_else(|| workspace.renderer.editor_bonds(&document, revision));
            bonds.sort_unstable_by_key(|bond| bond.atoms);
            match bonds.binary_search_by_key(&pair, |bond| bond.atoms) {
                Ok(i) if order == 0 => {
                    bonds.remove(i);
                }
                Ok(i) => bonds[i].order = order,
                Err(i) if order != 0 => bonds.insert(i, Bond { atoms: pair, order }),
                Err(_) => {}
            }
            document.bonds = Some(bonds);
        }
    }
    if composition_changed {
        let electrons = document
            .atoms
            .iter()
            .map(|atom| i32::from(atom.atomic_number))
            .sum::<i32>()
            - document.charge;
        if electrons > 0
            && (document.spin > electrons as u32
                || !(electrons as u32 - document.spin).is_multiple_of(2))
        {
            document.spin = electrons as u32 % 2;
        }
    }
    if workspace.project.document.as_ref() == Some(&document) {
        return Ok(());
    }
    document.source = None;
    commit(workspace, project, document)
}

fn commit(
    workspace: &mut Workspace,
    mut project: Project,
    document: Document,
) -> Result<(), String> {
    document.validate()?;
    project.settings = project.settings.for_spin(document.spin);
    project.calculation = None;
    project.document = Some(document);
    workspace.replace_project(project)
}

#[tauri::command]
pub fn edit_molecule(
    app: AppHandle,
    state: State<'_, AppState>,
    expected_revision: u64,
    edit: Edit,
) -> Result<WorkspaceSnapshot, String> {
    let snapshot = {
        let mut workspace = state.0.lock().map_err(|error| error.to_string())?;
        apply(&mut workspace, expected_revision, edit)?;
        workspace.snapshot()
    };
    publish(&app, snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(workspace: &mut Workspace, operation: Edit) {
        apply(workspace, workspace.document_revision, operation).unwrap();
    }

    #[test]
    fn edits_are_revision_checked_atomic_and_reversible() {
        let mut workspace = Workspace::default();
        for x in [0.0, 0.74, 1.48] {
            edit(
                &mut workspace,
                Edit::Add {
                    symbol: "H".into(),
                    position: [x, 0.0, 0.0],
                },
            );
        }
        assert_eq!(workspace.snapshot().spin, 1);
        let revision = workspace.document_revision;
        assert!(apply(&mut workspace, revision - 1, Edit::Delete { atom: 1 }).is_err());
        assert!(apply(
            &mut workspace,
            revision,
            Edit::Move {
                atom: 1,
                position: [0.74, 0.0, 0.0]
            }
        )
        .is_err());
        assert_eq!(workspace.document_revision, revision);
        edit(
            &mut workspace,
            Edit::Bond {
                first: 3,
                second: 2,
                order: 3,
            },
        );
        let manual = workspace.project.clone();
        let document = manual.document.as_ref().unwrap();
        assert_eq!(
            crate::renderer::scene::Scene::from_document(Some(document)).bond_instance_count(),
            8
        );
        edit(
            &mut workspace,
            Edit::Move {
                atom: 3,
                position: [3.0, 0.0, 0.0],
            },
        );
        edit(&mut workspace, Edit::Delete { atom: 1 });
        let edited = workspace.project.document.as_ref().unwrap();
        assert_eq!(
            edited.bonds.as_deref(),
            Some(
                &[Bond {
                    atoms: [0, 1],
                    order: 3
                }][..]
            )
        );
        assert_eq!(edited.spin, 0);
        workspace.history(false).unwrap();
        workspace.history(false).unwrap();
        assert_eq!(workspace.project, manual);
        workspace.history(true).unwrap();
        workspace.history(true).unwrap();
        edit(
            &mut workspace,
            Edit::Bond {
                first: 1,
                second: 2,
                order: 0,
            },
        );
        let path =
            std::env::temp_dir().join(format!("reactqm-editor-{}.json", uuid::Uuid::new_v4()));
        workspace.project.save(&path).unwrap();
        let reopened = Project::load(&path).unwrap();
        assert!(reopened.document.unwrap().bonds.unwrap().is_empty());
        edit(&mut workspace, Edit::Delete { atom: 2 });
        edit(&mut workspace, Edit::Delete { atom: 1 });
        assert_eq!(workspace.snapshot().atom_count, 0);
        workspace.project.save(&path).unwrap();
        assert!(Project::load(&path).unwrap().document.is_none());
        workspace.history(false).unwrap();
        assert_eq!(workspace.snapshot().atom_count, 1);
        std::fs::remove_file(path).unwrap();
    }
}
