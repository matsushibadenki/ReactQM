//! RDKit analysis belongs to Rust/Worker storage; IPC exposes one atom at a time.
use crate::{
    document::Document,
    jobs::{OwnedChild, WorkerConfig},
    state::AppState,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use tauri::Manager;

#[derive(Default)]
pub struct ValenceService {
    cache: Mutex<VecDeque<(String, Result<Analysis, String>)>>,
    pub stopping: AtomicBool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Analysis {
    schema_version: u32,
    input_hash: String,
    pub engine_version: String,
    pub status: String,
    pub reason: Option<String>,
    atoms: Vec<Atom>,
    bonds: Vec<Bond>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Atom {
    atomic_number: u8,
    valence: u32,
    formal_charge: i32,
    neighbors: usize,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Bond {
    a: usize,
    b: usize,
    order: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AtomValence {
    document_revision: u64,
    index: usize,
    status: String,
    reason: Option<String>,
    engine_version: String,
    valence: Option<u32>,
    formal_charge: Option<i32>,
    neighbors: Option<usize>,
}
impl Analysis {
    pub fn display_bonds(&self) -> Option<Vec<crate::renderer::scene::DisplayBond>> {
        (self.status == "inferred").then(|| {
            self.bonds
                .iter()
                .map(|bond| crate::renderer::scene::DisplayBond {
                    atoms: [bond.a, bond.b],
                    order: bond.order as u8,
                })
                .collect()
        })
    }
    fn validate(&self, document: &Document) -> Result<(), String> {
        let invalid = || "valence_protocol".to_string();
        if self.schema_version != 1
            || self.input_hash != document.fingerprint()
            || self.engine_version.is_empty()
            || self.engine_version.len() > 64
        {
            return Err(invalid());
        }
        if self.status != "inferred" {
            if !["unsupported", "failed"].contains(&self.status.as_str())
                || !self.atoms.is_empty()
                || !self.bonds.is_empty()
                || !matches!(
                    self.reason.as_deref(),
                    Some("open_shell" | "element" | "incomplete" | "bond_assignment")
                )
            {
                return Err(invalid());
            }
            return Ok(());
        }
        if self.reason.is_some()
            || self.atoms.len() != document.atoms.len()
            || self.bonds.len() > 2016
        {
            return Err(invalid());
        }
        let mut sums = vec![0; self.atoms.len()];
        let mut neighbors = vec![0; self.atoms.len()];
        let mut pairs = BTreeSet::new();
        for bond in &self.bonds {
            if bond.a >= sums.len()
                || bond.b >= sums.len()
                || bond.a == bond.b
                || !(1..=3).contains(&bond.order)
                || !pairs.insert((bond.a.min(bond.b), bond.a.max(bond.b)))
            {
                return Err(invalid());
            }
            sums[bond.a] += bond.order;
            sums[bond.b] += bond.order;
            neighbors[bond.a] += 1;
            neighbors[bond.b] += 1;
        }
        for (i, atom) in self.atoms.iter().enumerate() {
            if atom.atomic_number != document.atoms[i].atomic_number
                || atom.valence > 12
                || atom.valence != sums[i]
                || atom.neighbors != neighbors[i]
                || !(-8..=8).contains(&atom.formal_charge)
            {
                return Err(invalid());
            }
        }
        if self.atoms.iter().map(|a| a.formal_charge).sum::<i32>() != document.charge {
            return Err(invalid());
        }
        Ok(())
    }
    fn atom(&self, revision: u64, index: usize) -> AtomValence {
        let atom = self.atoms.get(index - 1);
        AtomValence {
            document_revision: revision,
            index,
            status: self.status.clone(),
            reason: self.reason.clone(),
            engine_version: self.engine_version.clone(),
            valence: atom.map(|a| a.valence),
            formal_charge: atom.map(|a| a.formal_charge),
            neighbors: atom.map(|a| a.neighbors),
        }
    }
}
pub fn analyze(
    document: &Document,
    config: &WorkerConfig,
    stopping: &AtomicBool,
    timeout: Duration,
) -> Result<Analysis, String> {
    document.validate()?;
    if !config.python.is_file() {
        return Err("worker_missing".into());
    }
    let bytes=serde_json::to_vec(&serde_json::json!({"schema_version":1,"input_hash":document.fingerprint(),"document":document})).map_err(|_|"valence_protocol")?;
    if bytes.len() > 32768 {
        return Err("valence_protocol".into());
    }
    let mut command = Command::new(&config.python);
    command
        .args(["-m", "reactqm_worker", "valence"])
        .current_dir(&config.root)
        .env("PYTHONPATH", config.root.join("src"))
        .env("OMP_NUM_THREADS", "1")
        .env("OPENBLAS_NUM_THREADS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = OwnedChild(command.spawn().map_err(|_| "worker_start")?);
    let mut input = child.0.stdin.take().ok_or("valence_protocol")?;
    let output = child.0.stdout.take().ok_or("valence_protocol")?;
    let writer = std::thread::spawn(move || input.write_all(&bytes));
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.take(65537).read_to_end(&mut bytes).map(|_| bytes)
    });
    let started = Instant::now();
    let exit = loop {
        if stopping.load(Ordering::SeqCst) {
            break Err("queue_stopping".to_string());
        }
        if started.elapsed() > timeout {
            break Err("valence_timeout".to_string());
        }
        match child.0.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err("valence_failed".into())
                }
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break Err("valence_failed".into()),
        }
    };
    // Also closes inherited pipes on Unix; always reap before joining reader/writer.
    drop(child);
    let written = writer.join();
    let read = reader.join();
    exit?;
    written
        .map_err(|_| "valence_protocol")?
        .map_err(|_| "valence_protocol")?;
    let bytes = read
        .map_err(|_| "valence_protocol")?
        .map_err(|_| "valence_protocol")?;
    if bytes.len() > 65536 {
        return Err("valence_protocol".into());
    }
    let result: Analysis = serde_json::from_slice(&bytes).map_err(|_| "valence_protocol")?;
    result.validate(document)?;
    Ok(result)
}
async fn perceive(
    app: tauri::AppHandle,
    expected_revision: u64,
    retry: bool,
) -> Result<Analysis, String> {
    let document = {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|e| e.to_string())?;
        if expected_revision != workspace.document_revision {
            return Err("stale_document".into());
        }
        workspace.project.document.clone().ok_or("no_document")?
    };
    tauri::async_runtime::spawn_blocking(move || {
        let service = app.state::<ValenceService>();
        let mut cache = service.cache.lock().map_err(|e| e.to_string())?;
        if service.stopping.load(Ordering::SeqCst) {
            return Err("queue_stopping".into());
        }
        let key = document.fingerprint();
        if retry {
            cache.retain(|(hash, _)| hash != &key);
        }
        let result = if let Some((_, result)) = cache.iter().find(|(hash, _)| hash == &key) {
            result.clone()
        } else {
            let result = analyze(
                &document,
                &WorkerConfig::development(),
                &service.stopping,
                Duration::from_secs(10),
            );
            if cache.len() == 8 {
                cache.pop_front();
            }
            cache.push_back((key, result.clone()));
            result
        };
        let state = app.state::<AppState>();
        let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
        if workspace.document_revision != expected_revision {
            return Err("stale_document".into());
        }
        let (status, version, bonds) = match &result {
            Ok(analysis) => (
                analysis.status.clone(),
                Some(analysis.engine_version.clone()),
                analysis.display_bonds(),
            ),
            Err(_) => ("failed".into(), None, None),
        };
        workspace.renderer.apply_bond_analysis(
            &document,
            expected_revision,
            status,
            version,
            bonds,
        )?;
        drop(cache);
        result
    })
    .await
    .map_err(|e| e.to_string())?
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BondAnalysis {
    document_revision: u64,
    status: String,
}
#[tauri::command]
pub async fn analyze_bonds(
    app: tauri::AppHandle,
    expected_revision: u64,
    retry: Option<bool>,
) -> Result<BondAnalysis, String> {
    let analysis = perceive(app, expected_revision, retry.unwrap_or(false)).await?;
    Ok(BondAnalysis {
        document_revision: expected_revision,
        status: analysis.status,
    })
}
#[tauri::command]
pub async fn atom_valence(
    app: tauri::AppHandle,
    expected_revision: u64,
    index: usize,
    retry: Option<bool>,
) -> Result<AtomValence, String> {
    {
        let state = app.state::<AppState>();
        let workspace = state.0.lock().map_err(|e| e.to_string())?;
        if expected_revision != workspace.document_revision {
            return Err("stale_document".into());
        }
        let document = workspace.project.document.as_ref().ok_or("no_document")?;
        if index == 0 || index > document.atoms.len() {
            return Err("valence_protocol".into());
        }
    }
    Ok(perceive(app, expected_revision, retry.unwrap_or(false))
        .await?
        .atom(expected_revision, index))
}
pub fn shutdown(app: &tauri::AppHandle) {
    let service = app.state::<ValenceService>();
    service.stopping.store(true, Ordering::SeqCst);
    drop(service.cache.lock()); // Wait for cancellation / process reap without the document lock.
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_rdkit_bond_orders_and_bounded_protocol() {
        let document =
            Document::from_xyz("3\nCO2\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0", 0, 0).unwrap();
        let mut result = analyze(
            &document,
            &WorkerConfig::development(),
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        let atom = result.atom(7, 2);
        assert_eq!(atom.valence, Some(4));
        assert_eq!(atom.neighbors, Some(2));
        assert_eq!(atom.document_revision, 7);
        result.atoms[1].valence = 2;
        assert!(result.validate(&document).is_err());
        result.atoms[1].valence = 4;
        result.input_hash = "foreign".into();
        assert!(result.validate(&document).is_err());
        assert_eq!(
            analyze(
                &document,
                &WorkerConfig::development(),
                &AtomicBool::new(true),
                Duration::from_secs(10)
            )
            .unwrap_err(),
            "queue_stopping"
        );
        assert_eq!(
            analyze(
                &document,
                &WorkerConfig::development(),
                &AtomicBool::new(false),
                Duration::ZERO
            )
            .unwrap_err(),
            "valence_timeout"
        );
    }
}
