//! Bounded, versioned panel arrangement. Scientific documents never live in a window.
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Panel {
    Result,
    History,
}
impl Panel {
    pub const ALL: [Self; 2] = [Self::Result, Self::History];
    pub fn label(self) -> &'static str {
        match self {
            Self::Result => "panel-result",
            Self::History => "panel-history",
        }
    }
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|panel| panel.label() == label)
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dock {
    Right,
    Bottom,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Zone {
    Right,
    Bottom,
    Floating,
}
impl From<Dock> for Zone {
    fn from(dock: Dock) -> Self {
        match dock {
            Dock::Right => Self::Right,
            Dock::Bottom => Self::Bottom,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Tabs,
    Split,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Placement {
    pub id: Panel,
    pub zone: Zone,
    pub home: Dock,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PanelLayout {
    pub schema_version: u32,
    pub revision: u64,
    pub panels: Vec<Placement>,
    pub right_active: Panel,
    pub bottom_active: Panel,
    pub right_mode: Mode,
    pub bottom_mode: Mode,
    pub right_width: u32,
}
impl Default for PanelLayout {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            panels: Panel::ALL
                .into_iter()
                .map(|id| Placement {
                    id,
                    zone: Zone::Right,
                    home: Dock::Right,
                })
                .collect(),
            right_active: Panel::Result,
            bottom_active: Panel::History,
            right_mode: Mode::Split,
            bottom_mode: Mode::Tabs,
            right_width: 42,
        }
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Move {
        panel: Panel,
        zone: Zone,
        before: Option<Panel>,
    },
    Activate {
        panel: Panel,
    },
    Mode {
        dock: Dock,
        mode: Mode,
    },
    Resize {
        width: u32,
    },
    Reset,
}
impl PanelLayout {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.panels.len() != 2
            || !(25..=60).contains(&self.right_width)
            || Panel::ALL
                .iter()
                .any(|id| self.panels.iter().filter(|p| p.id == *id).count() != 1)
        {
            return Err("layout_invalid".into());
        }
        Ok(())
    }
    pub fn zone(&self, panel: Panel) -> Zone {
        self.panels.iter().find(|p| p.id == panel).unwrap().zone
    }
    pub fn redock(&self, panel: Panel) -> Self {
        let home = self.panels.iter().find(|p| p.id == panel).unwrap().home;
        self.apply(Action::Move {
            panel,
            zone: home.into(),
            before: None,
        })
        .unwrap()
    }
    pub fn apply(&self, action: Action) -> Result<Self, String> {
        let mut next = self.clone();
        match action {
            Action::Move {
                panel,
                zone,
                before,
            } => {
                if before == Some(panel) {
                    return Ok(next);
                }
                let index = next
                    .panels
                    .iter()
                    .position(|p| p.id == panel)
                    .ok_or("layout_invalid")?;
                let mut placement = next.panels.remove(index);
                placement.zone = zone;
                match zone {
                    Zone::Right => {
                        placement.home = Dock::Right;
                        next.right_active = panel;
                    }
                    Zone::Bottom => {
                        placement.home = Dock::Bottom;
                        next.bottom_active = panel;
                    }
                    Zone::Floating => {}
                }
                let index = if let Some(before) = before {
                    next.panels
                        .iter()
                        .position(|p| p.id == before && p.zone == zone)
                        .ok_or("layout_invalid")?
                } else {
                    next.panels.len()
                };
                next.panels.insert(index, placement);
            }
            Action::Activate { panel } => match next.zone(panel) {
                Zone::Right => next.right_active = panel,
                Zone::Bottom => next.bottom_active = panel,
                Zone::Floating => {}
            },
            Action::Mode { dock, mode } => match dock {
                Dock::Right => next.right_mode = mode,
                Dock::Bottom => next.bottom_mode = mode,
            },
            Action::Resize { width } => next.right_width = width,
            Action::Reset => next = Self::default(),
        }
        next.revision = self.revision.checked_add(1).ok_or("layout_invalid")?;
        next.validate()?;
        Ok(next)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        crate::project::atomic_json(path, self)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let mut bytes = Vec::new();
        File::open(path)
            .map_err(|e| format!("storage: {e}"))?
            .take(8193)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("storage: {e}"))?;
        if bytes.len() > 8192 {
            return Err("layout_invalid".into());
        }
        let layout: Self = serde_json::from_slice(&bytes).map_err(|_| "layout_invalid")?;
        layout.validate()?;
        Ok(layout)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_tabbing_and_redocking_preserve_exactly_one_copy() {
        let layout = PanelLayout::default()
            .apply(Action::Move {
                panel: Panel::History,
                zone: Zone::Bottom,
                before: None,
            })
            .unwrap();
        let floating = layout
            .apply(Action::Move {
                panel: Panel::History,
                zone: Zone::Floating,
                before: None,
            })
            .unwrap();
        assert_eq!(
            floating.redock(Panel::History).zone(Panel::History),
            Zone::Bottom
        );
        let tabbed = layout
            .apply(Action::Move {
                panel: Panel::Result,
                zone: Zone::Bottom,
                before: Some(Panel::History),
            })
            .unwrap()
            .apply(Action::Mode {
                dock: Dock::Bottom,
                mode: Mode::Tabs,
            })
            .unwrap();
        assert_eq!(tabbed.panels[0].id, Panel::Result);
        assert_eq!(tabbed.bottom_active, Panel::Result);
        assert_eq!(tabbed.panels.len(), 2);
        assert_eq!(
            tabbed.apply(Action::Reset).unwrap().revision,
            tabbed.revision + 1
        );
    }
    #[test]
    fn saved_arrangements_roundtrip_and_invalid_changes_leave_the_file_intact() {
        let dir = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        let path = dir.join("layout.json");
        let layout = PanelLayout::default()
            .apply(Action::Move {
                panel: Panel::History,
                zone: Zone::Floating,
                before: None,
            })
            .unwrap();
        layout.save(&path).unwrap();
        assert_eq!(PanelLayout::load(&path).unwrap(), layout);
        assert!(layout.apply(Action::Resize { width: 61 }).is_err());
        let mut invalid = layout.clone();
        invalid.panels[0].id = Panel::History;
        assert!(invalid.save(&path).is_err());
        assert_eq!(PanelLayout::load(&path).unwrap(), layout);
        invalid = layout.clone();
        invalid.schema_version = 2;
        assert!(invalid.validate().is_err());
        std::fs::write(&path, b"{broken").unwrap();
        assert!(PanelLayout::load(&path).is_err());
        std::fs::write(&path, vec![b' '; 8193]).unwrap();
        assert!(PanelLayout::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
