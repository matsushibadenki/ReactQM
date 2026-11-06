use crate::{document::Document, renderer::Renderer};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
pub enum Locale {
    #[serde(rename = "en")]
    En,
    #[default]
    #[serde(rename = "ja")]
    Ja,
    #[serde(rename = "zh-CN")]
    ZhCn,
}

/// Layout authority will remain here when native floating panels are added.
#[derive(Default)]
pub struct PanelLayout {
    pub floating_panel_ids: Vec<String>,
}

#[derive(Default)]
pub struct Workspace {
    pub document: Document,
    pub renderer: Renderer,
    pub layout: PanelLayout,
    pub locale: Locale,
}

#[derive(Default)]
pub struct AppState(pub Mutex<Workspace>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub locale: Locale,
    pub atom_count: usize,
    pub renderer_ready: bool,
}

impl Workspace {
    pub fn snapshot(&self) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            locale: self.locale,
            atom_count: self.document.atoms.len(),
            renderer_ready: self.renderer.context.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Atom;

    #[test]
    fn snapshot_reads_authoritative_document() {
        let mut workspace = Workspace::default();
        workspace.document.atoms.push(Atom {
            atomic_number: 1,
            position_angstrom: [0.0; 3],
        });
        assert_eq!(workspace.snapshot().atom_count, 1);
        assert!(!workspace.snapshot().renderer_ready);
    }
}
