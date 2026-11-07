//! Durable calculation records. Only this service owns SQLite and the workspace lease.
use crate::{
    jobs::JobRequest,
    project::{atomic_json, Calculation, Project},
    state::JobSnapshot,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const PAGE_SIZE: usize = 20;
const MAX_ARTIFACT: u64 = 64 * 1024 * 1024;
const FILES: [&str; 3] = ["input.json", "result.json", "result.h5"];

pub struct CatalogState(pub Result<Catalog, String>);
impl CatalogState {
    pub fn get(&self) -> Result<&Catalog, String> {
        self.0.as_ref().map_err(Clone::clone)
    }
}
pub struct Catalog {
    root: PathBuf,
    connection: Mutex<Connection>,
    _lease: File,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobRecord {
    pub sequence: i64,
    pub id: String,
    pub name: String,
    pub formula: String,
    pub status: String,
    pub created_at: i64,
    pub error: Option<String>,
    pub energy_hartree: Option<f64>,
    pub retry_of: Option<String>,
    pub legacy: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPage {
    pub items: Vec<JobRecord>,
    pub next_cursor: Option<i64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub sequence: i64,
    pub id: String,
    pub name: String,
    pub formula: String,
    pub status: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuePage {
    pub paused: bool,
    pub waiting: usize,
    pub active: Option<QueueItem>,
    pub items: Vec<QueueItem>,
    pub next_cursor: Option<i64>,
}
pub enum Claim {
    Empty,
    Rejected {
        id: String,
        error: String,
    },
    Ready {
        request: JobRequest,
        directory: PathBuf,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    job_id: String,
    input_hash: String,
    artifacts: Vec<Artifact>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    name: String,
    bytes: u64,
    sha256: String,
}
fn storage(error: impl std::fmt::Display) -> String {
    format!("storage: {error}")
}
fn timestamp() -> Result<i64, String> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(storage)?
            .as_millis(),
    )
    .map_err(storage)
}
fn check_id(id: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(id)
        .map_err(|_| "job_invalid")?
        .to_string()
        != id
    {
        return Err("job_invalid".into());
    }
    Ok(())
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let metadata = fs::symlink_metadata(path).map_err(storage)?;
    if !metadata.is_file() || metadata.len() > 131072 {
        return Err("artifact_invalid".into());
    }
    let bytes = fs::read(path).map_err(storage)?;
    if bytes.len() > 131072 {
        return Err("artifact_invalid".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "artifact_invalid".into())
}
fn digest(path: &Path) -> Result<(u64, String), String> {
    let metadata = fs::symlink_metadata(path).map_err(storage)?;
    if !metadata.is_file() || metadata.len() > MAX_ARTIFACT {
        return Err("artifact_invalid".into());
    }
    let mut file = File::open(path).map_err(storage)?.take(MAX_ARTIFACT + 1);
    let mut hash = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let read = file.read(&mut buffer).map_err(storage)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hash.update(&buffer[..read]);
    }
    if bytes > MAX_ARTIFACT {
        return Err("artifact_invalid".into());
    }
    Ok((bytes, format!("{:x}", hash.finalize())))
}
impl Catalog {
    pub fn open(root: &Path) -> Result<Self, String> {
        fs::create_dir_all(root.join("jobs")).map_err(storage)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("workspace.lock"))
            .map_err(storage)?;
        lease.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => "workspace_in_use".into(),
            other => storage(other),
        })?;
        let connection = Connection::open(root.join("jobs.sqlite3")).map_err(storage)?;
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(storage)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage)?;
        if version > 2 {
            return Err("catalog_version".into());
        }
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS jobs (
              sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
              request_json TEXT NOT NULL, name TEXT NOT NULL, formula TEXT NOT NULL,
              status TEXT NOT NULL, created_at INTEGER NOT NULL, error TEXT,
              energy_hartree REAL, retry_of TEXT, receipt_sha256 TEXT, legacy INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS catalog_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS queue_status_sequence ON jobs(status,sequence);
            PRAGMA user_version=2; COMMIT;").map_err(storage)?;
        let catalog = Self {
            root: root.to_path_buf(),
            connection: Mutex::new(connection),
            _lease: lease,
        };
        catalog.import_legacy()?;
        catalog.recover()?;
        // Every application start requires an explicit resume; queued UUIDs and order survive.
        catalog.set_queue_paused(true)?;
        Ok(catalog)
    }
    fn directory(&self, id: &str) -> Result<PathBuf, String> {
        check_id(id)?;
        let directory = self.root.join("jobs").join(id);
        if let Ok(metadata) = fs::symlink_metadata(&directory) {
            if !metadata.is_dir() {
                return Err("artifact_invalid".into());
            }
        }
        Ok(directory)
    }
    fn insert(
        &self,
        request: &JobRequest,
        status: &str,
        retry_of: Option<&str>,
        legacy: bool,
        created: i64,
    ) -> Result<(), String> {
        request.validate()?;
        self.connection.lock().map_err(storage)?.execute(
            "INSERT INTO jobs (id,request_json,name,formula,status,created_at,retry_of,legacy) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![request.job_id,serde_json::to_string(request).map_err(storage)?,request.document.name,request.document.formula(),status,created,retry_of,legacy]
        ).map_err(storage)?;
        Ok(())
    }
    /// Persist a running attempt before any child process can be launched.
    pub fn begin(&self, request: &JobRequest, retry_of: Option<&str>) -> Result<PathBuf, String> {
        request.validate()?;
        let directory = self.directory(&request.job_id)?;
        fs::create_dir(&directory).map_err(storage)?;
        atomic_json(&directory.join("input.json"), request)?;
        self.insert(request, "running", retry_of, false, timestamp()?)?;
        Ok(directory)
    }
    /// An immutable input file and a durable queued row; no process is started here.
    pub fn enqueue(&self, request: &JobRequest, retry_of: Option<&str>) -> Result<(), String> {
        request.validate()?;
        let directory = self.directory(&request.job_id)?;
        fs::create_dir(&directory).map_err(storage)?;
        atomic_json(&directory.join("input.json"), request)?;
        let changed=self.connection.lock().map_err(storage)?.execute(
            "INSERT INTO jobs (id,request_json,name,formula,status,created_at,retry_of,legacy) SELECT ?1,?2,?3,?4,'queued',?5,?6,0 WHERE (SELECT COUNT(*) FROM jobs WHERE status='queued')<100",
            params![request.job_id,serde_json::to_string(request).map_err(storage)?,request.document.name,request.document.formula(),timestamp()?,retry_of]
        ).map_err(storage)?;
        if changed != 1 {
            let _ = fs::remove_file(directory.join("input.json"));
            let _ = fs::remove_dir(&directory);
            return Err("queue_full".into());
        }
        Ok(())
    }
    pub fn set_queue_paused(&self, paused: bool) -> Result<(), String> {
        self.connection.lock().map_err(storage)?.execute("INSERT INTO catalog_meta (key,value) VALUES ('queue_paused',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[if paused { "1" } else { "0" }]).map_err(storage)?;
        Ok(())
    }
    pub fn queue_page(&self, after: Option<i64>) -> Result<QueuePage, String> {
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection.transaction().map_err(storage)?;
        let paused = transaction
            .query_row(
                "SELECT value='1' FROM catalog_meta WHERE key='queue_paused'",
                [],
                |row| row.get(0),
            )
            .map_err(storage)?;
        let waiting = transaction
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE status='queued'",
                [],
                |row| row.get::<_, usize>(0),
            )
            .map_err(storage)?;
        let map = |row: &rusqlite::Row<'_>| {
            Ok(QueueItem {
                sequence: row.get(0)?,
                id: row.get(1)?,
                name: row.get(2)?,
                formula: row.get(3)?,
                status: row.get(4)?,
            })
        };
        let active=transaction.query_row("SELECT sequence,id,name,formula,status FROM jobs WHERE status IN ('running','cancelling') ORDER BY sequence LIMIT 1",[],map).optional().map_err(storage)?;
        let mut items = {
            let mut statement=transaction.prepare("SELECT sequence,id,name,formula,status FROM jobs WHERE status='queued' AND (?1 IS NULL OR sequence>?1) ORDER BY sequence LIMIT ?2").map_err(storage)?;
            let rows = statement
                .query_map(params![after, (PAGE_SIZE + 1) as i64], map)
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            rows
        };
        let next_cursor = if items.len() > PAGE_SIZE {
            items.truncate(PAGE_SIZE);
            items.last().map(|item| item.sequence)
        } else {
            None
        };
        transaction.commit().map_err(storage)?;
        Ok(QueuePage {
            paused,
            waiting,
            active,
            items,
            next_cursor,
        })
    }
    /// Claim and queued cancellation serialize through the same SQLite transaction boundary.
    pub fn claim_next(&self) -> Result<Claim, String> {
        let mut connection = self.connection.lock().map_err(storage)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage)?;
        let blocked:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM catalog_meta WHERE key='queue_paused' AND value='1') OR EXISTS(SELECT 1 FROM jobs WHERE status IN ('running','cancelling'))",[],|r|r.get(0)).map_err(storage)?;
        if blocked {
            return Ok(Claim::Empty);
        }
        let next: Option<(String, String)> = transaction
            .query_row(
                "SELECT id,request_json FROM jobs WHERE status='queued' ORDER BY sequence LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage)?;
        let Some((id, json)) = next else {
            return Ok(Claim::Empty);
        };
        let verified = (|| {
            let request: JobRequest = serde_json::from_str(&json).map_err(|_| "job_invalid")?;
            request.validate()?;
            if request.job_id != id {
                return Err("job_invalid".into());
            }
            let directory = self.directory(&id)?;
            let saved: JobRequest = read_json(&directory.join("input.json"))?;
            if serde_json::to_vec(&saved).map_err(storage)?
                != serde_json::to_vec(&request).map_err(storage)?
            {
                return Err("artifact_invalid".into());
            }
            Ok((request, directory))
        })();
        let result = match verified {
            Ok((request, directory)) => {
                transaction
                    .execute(
                        "UPDATE jobs SET status='running' WHERE id=?1 AND status='queued'",
                        [&id],
                    )
                    .map_err(storage)?;
                Claim::Ready { request, directory }
            }
            Err(error) => {
                transaction
                    .execute(
                        "UPDATE jobs SET status='failed',error=?2 WHERE id=?1 AND status='queued'",
                        params![id, error],
                    )
                    .map_err(storage)?;
                Claim::Rejected { id, error }
            }
        };
        transaction.commit().map_err(storage)?;
        Ok(result)
    }
    pub fn cancel_queued(&self, id: &str) -> Result<(), String> {
        check_id(id)?;
        let changed = self
            .connection
            .lock()
            .map_err(storage)?
            .execute(
                "UPDATE jobs SET status='cancelled',error=NULL WHERE id=?1 AND status='queued'",
                [id],
            )
            .map_err(storage)?;
        if changed != 1 {
            return Err("job_not_queued".into());
        }
        let _ = atomic_json(
            &self.directory(id)?.join("status.json"),
            &JobSnapshot {
                id: id.into(),
                status: "cancelled".into(),
                cycle: 0,
                error: None,
            },
        );
        Ok(())
    }
    pub fn page(&self, before: Option<i64>) -> Result<JobPage, String> {
        let connection = self.connection.lock().map_err(storage)?;
        let mut statement=connection.prepare("SELECT sequence,id,name,formula,status,created_at,error,energy_hartree,retry_of,legacy FROM jobs WHERE (?1 IS NULL OR sequence < ?1) ORDER BY sequence DESC LIMIT ?2").map_err(storage)?;
        let mut items = statement
            .query_map(params![before, (PAGE_SIZE + 1) as i64], |row| {
                Ok(JobRecord {
                    sequence: row.get(0)?,
                    id: row.get(1)?,
                    name: row.get(2)?,
                    formula: row.get(3)?,
                    status: row.get(4)?,
                    created_at: row.get(5)?,
                    error: row.get(6)?,
                    energy_hartree: row.get(7)?,
                    retry_of: row.get(8)?,
                    legacy: row.get(9)?,
                })
            })
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        let next_cursor = if items.len() > PAGE_SIZE {
            items.truncate(PAGE_SIZE);
            items.last().map(|row| row.sequence)
        } else {
            None
        };
        Ok(JobPage { items, next_cursor })
    }
    fn row(&self, id: &str) -> Result<(JobRequest, String, Option<String>), String> {
        check_id(id)?;
        let row: Option<(String, String, Option<String>)> = self
            .connection
            .lock()
            .map_err(storage)?
            .query_row(
                "SELECT request_json,status,receipt_sha256 FROM jobs WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(storage)?;
        let (json, status, receipt) = row.ok_or("job_missing")?;
        let request: JobRequest = serde_json::from_str(&json).map_err(|_| "job_invalid")?;
        request.validate()?;
        if request.job_id != id {
            return Err("job_invalid".into());
        }
        Ok((request, status, receipt))
    }
    pub fn retry_source(&self, id: &str) -> Result<JobRequest, String> {
        let (request, status, _) = self.row(id)?;
        if ["queued", "running", "cancelling"].contains(&status.as_str()) {
            return Err("busy".into());
        }
        Ok(request)
    }
    fn make_receipt(
        &self,
        request: &JobRequest,
        calculation: &Calculation,
    ) -> Result<String, String> {
        let directory = self.directory(&request.job_id)?;
        let input: JobRequest = read_json(&directory.join("input.json"))?;
        if serde_json::to_vec(&input).map_err(storage)?
            != serde_json::to_vec(request).map_err(storage)?
        {
            return Err("artifact_invalid".into());
        }
        let result: Calculation = read_json(&directory.join("result.json"))?;
        result.validate(&request.document)?;
        if &result != calculation || result.job_id != request.job_id {
            return Err("artifact_invalid".into());
        }
        let mut header = [0u8; 8];
        File::open(directory.join("result.h5"))
            .map_err(storage)?
            .read_exact(&mut header)
            .map_err(storage)?;
        if header != *b"\x89HDF\r\n\x1a\n" {
            return Err("artifact_invalid".into());
        }
        let artifacts = FILES
            .iter()
            .map(|name| {
                let (bytes, sha256) = digest(&directory.join(name))?;
                Ok(Artifact {
                    name: (*name).into(),
                    bytes,
                    sha256,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        atomic_json(
            &directory.join("completion.json"),
            &Receipt {
                schema_version: 1,
                job_id: request.job_id.clone(),
                input_hash: request.input_hash.clone(),
                artifacts,
            },
        )?;
        Ok(digest(&directory.join("completion.json"))?.1)
    }
    fn verify(
        &self,
        request: &JobRequest,
        expected_receipt: Option<&str>,
    ) -> Result<Calculation, String> {
        let directory = self.directory(&request.job_id)?;
        let receipt_path = directory.join("completion.json");
        if let Some(expected) = expected_receipt {
            if digest(&receipt_path)?.1 != expected {
                return Err("artifact_invalid".into());
            }
        }
        let receipt: Receipt = read_json(&receipt_path)?;
        if receipt.schema_version != 1
            || receipt.job_id != request.job_id
            || receipt.input_hash != request.input_hash
            || receipt.artifacts.len() != FILES.len()
        {
            return Err("artifact_invalid".into());
        }
        for (artifact, name) in receipt.artifacts.iter().zip(FILES) {
            if artifact.name != name
                || digest(&directory.join(name))? != (artifact.bytes, artifact.sha256.clone())
            {
                return Err("artifact_invalid".into());
            }
        }
        let input: JobRequest = read_json(&directory.join("input.json"))?;
        if serde_json::to_vec(&input).map_err(storage)?
            != serde_json::to_vec(request).map_err(storage)?
        {
            return Err("artifact_invalid".into());
        }
        let result: Calculation = read_json(&directory.join("result.json"))?;
        result.validate(&request.document)?;
        if result.job_id != request.job_id {
            return Err("artifact_invalid".into());
        }
        Ok(result)
    }
    pub fn complete(&self, request: &JobRequest, calculation: &Calculation) -> Result<(), String> {
        let status = self.row(&request.job_id)?.1;
        if status == "cancelling" {
            return Err("cancelled".into());
        }
        if status != "running" {
            return Err("job_invalid".into());
        }
        let receipt = match self.make_receipt(request, calculation) {
            Ok(receipt) => receipt,
            Err(error) => {
                if self.row(&request.job_id)?.1 == "cancelling" {
                    return Err("cancelled".into());
                }
                self.fail(&request.job_id, "failed", Some("artifact_invalid"))?;
                return Err(error);
            }
        };
        let changed=self.connection.lock().map_err(storage)?.execute("UPDATE jobs SET status='succeeded',error=NULL,energy_hartree=?2,receipt_sha256=?3 WHERE id=?1 AND status='running'",params![request.job_id,calculation.energy_hartree,receipt]).map_err(storage)?;
        if changed != 1 {
            return Err("cancelled".into());
        }
        // SQLite and the receipt are authoritative; this compatibility file is best effort.
        let _ = atomic_json(
            &self.directory(&request.job_id)?.join("status.json"),
            &JobSnapshot {
                id: request.job_id.clone(),
                status: "succeeded".into(),
                cycle: 0,
                error: None,
            },
        );
        Ok(())
    }
    pub fn request_cancel(&self, id: &str) -> Result<(), String> {
        let changed = self
            .connection
            .lock()
            .map_err(storage)?
            .execute(
                "UPDATE jobs SET status='cancelling' WHERE id=?1 AND status='running'",
                [id],
            )
            .map_err(storage)?;
        if changed != 1 {
            return Err("no_job".into());
        }
        Ok(())
    }
    pub fn fail(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), String> {
        if !["failed", "cancelled", "timed_out", "interrupted", "corrupt"].contains(&status) {
            return Err("job_invalid".into());
        }
        self.update(id, status, error, None, None)?;
        let _ = atomic_json(
            &self.directory(id)?.join("status.json"),
            &JobSnapshot {
                id: id.into(),
                status: status.into(),
                cycle: 0,
                error: error.map(str::to_owned),
            },
        );
        Ok(())
    }
    fn update(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        energy: Option<f64>,
        receipt: Option<&str>,
    ) -> Result<(), String> {
        let changed=self.connection.lock().map_err(storage)?.execute("UPDATE jobs SET status=?2,error=?3,energy_hartree=?4,receipt_sha256=?5 WHERE id=?1",params![id,status,error,energy,receipt]).map_err(storage)?;
        if changed != 1 {
            return Err("job_missing".into());
        }
        Ok(())
    }
    pub fn restore(&self, id: &str) -> Result<Project, String> {
        let (request, status, receipt) = self.row(id)?;
        if status != "succeeded" {
            return Err("job_not_succeeded".into());
        }
        let result = self.verify(
            &request,
            Some(receipt.as_deref().ok_or("artifact_invalid")?),
        );
        match result {
            Ok(calculation) => Ok(Project {
                document: Some(request.document),
                calculation: Some(calculation),
            }),
            Err(_) => {
                self.update(id, "corrupt", Some("artifact_invalid"), None, None)?;
                Err("artifact_invalid".into())
            }
        }
    }
    fn recover(&self) -> Result<(), String> {
        let ids = {
            let connection = self.connection.lock().map_err(storage)?;
            let mut statement = connection
                .prepare("SELECT id FROM jobs WHERE status IN ('running','cancelling')")
                .map_err(storage)?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(storage)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(storage)?;
            rows
        };
        for id in ids {
            let (request, status, _) = self.row(&id)?;
            if status == "cancelling" {
                self.fail(&id, "cancelled", None)?;
                continue;
            }
            // Only our receipt, written after successful child exit, authorizes success recovery.
            match self.verify(&request, None) {
                Ok(calculation) => {
                    let receipt = digest(&self.directory(&id)?.join("completion.json"))?.1;
                    self.update(
                        &id,
                        "succeeded",
                        None,
                        Some(calculation.energy_hartree),
                        Some(&receipt),
                    )?;
                }
                Err(_) => self.fail(&id, "interrupted", Some("job_interrupted"))?,
            }
        }
        Ok(())
    }
    fn import_legacy(&self) -> Result<(), String> {
        let done: bool = self
            .connection
            .lock()
            .map_err(storage)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM catalog_meta WHERE key='legacy_imported')",
                [],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if done {
            return Ok(());
        }
        let mut entries = fs::read_dir(self.root.join("jobs"))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        entries.sort_by_key(|entry| {
            (
                entry.metadata().and_then(|m| m.modified()).ok(),
                entry.file_name(),
            )
        });
        for entry in entries {
            let id = entry.file_name().to_string_lossy().into_owned();
            if check_id(&id).is_err() || !entry.file_type().map_err(storage)?.is_dir() {
                continue;
            }
            let existing = self.row(&id).ok();
            if existing
                .as_ref()
                .is_some_and(|(_, status, receipt)| status != "interrupted" || receipt.is_some())
            {
                continue;
            }
            let Ok(request) = read_json::<JobRequest>(&entry.path().join("input.json")) else {
                continue;
            };
            if request.job_id != id || request.validate().is_err() {
                continue;
            }
            let created = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis().min(i64::MAX as u128) as i64);
            let has_receipt = fs::symlink_metadata(entry.path().join("completion.json")).is_ok();
            if existing.is_none() {
                self.insert(&request, "interrupted", None, !has_receipt, created)?;
            }
            let status = read_json::<JobSnapshot>(&entry.path().join("status.json"))
                .ok()
                .filter(|s| s.id == id);
            if let Some(status) = status.as_ref().filter(|s| {
                ["failed", "cancelled", "timed_out", "cancelling"].contains(&s.status.as_str())
            }) {
                let terminal = if status.status == "cancelling" {
                    "cancelled"
                } else {
                    &status.status
                };
                self.update(&id, terminal, status.error.as_deref(), None, None)?;
            } else if has_receipt {
                // Reindexing must preserve an existing baseline, never rehash changed files as new truth.
                match self.verify(&request, None) {
                    Ok(calculation) => {
                        let receipt = digest(&entry.path().join("completion.json"))?.1;
                        self.update(
                            &id,
                            "succeeded",
                            None,
                            Some(calculation.energy_hartree),
                            Some(&receipt),
                        )?;
                    }
                    Err(_) => self.update(&id, "corrupt", Some("artifact_invalid"), None, None)?,
                }
            } else if status.as_ref().is_some_and(|s| s.status == "succeeded") {
                if let Ok(calculation) = read_json::<Calculation>(&entry.path().join("result.json"))
                {
                    self.update(&id, "running", None, None, None)?;
                    if self.complete(&request, &calculation).is_ok() {
                        continue;
                    }
                }
                self.update(&id, "corrupt", Some("artifact_invalid"), None, None)?;
            } else {
                self.update(&id, "interrupted", Some("job_interrupted"), None, None)?;
            }
        }
        self.connection
            .lock()
            .map_err(storage)?
            .execute(
                "INSERT INTO catalog_meta (key,value) VALUES ('legacy_imported','1')",
                [],
            )
            .map_err(storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!("reactqm-catalog-{}", uuid::Uuid::new_v4()))
    }
    fn request() -> JobRequest {
        JobRequest::new(
            crate::document::Document::from_xyz("2\nH2\nH 0 0 0\nH 0 0 0.74", 0, 0).unwrap(),
            7,
        )
        .unwrap()
    }
    // Storage fixtures only. The separate catalog_probe uses real PySCF/HDF5 artifacts.
    fn outputs(root: &Path, request: &JobRequest) -> Calculation {
        let result = Calculation {
            job_id: request.job_id.clone(),
            input_hash: request.input_hash.clone(),
            method: request.method.clone(),
            basis: request.basis.clone(),
            energy_hartree: -1.1167593074,
            converged: true,
            engine_version: "storage-fixture".into(),
            elapsed_seconds: 0.2,
        };
        let directory = root.join("jobs").join(&request.job_id);
        atomic_json(&directory.join("result.json"), &result).unwrap();
        fs::write(directory.join("result.h5"), b"\x89HDF\r\n\x1a\nfixture").unwrap();
        result
    }
    #[test]
    fn durable_restore_and_retry_keep_input_and_settings() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let mut input = request();
        input.max_cycle = 17;
        input.conv_tol = 1e-10;
        store.begin(&input, None).unwrap();
        let calculation = outputs(&root, &input);
        store.complete(&input, &calculation).unwrap();
        drop(store);
        let store = Catalog::open(&root).unwrap();
        assert_eq!(
            store.restore(&input.job_id).unwrap(),
            Project {
                document: Some(input.document.clone()),
                calculation: Some(calculation)
            }
        );
        let next = store
            .retry_source(&input.job_id)
            .unwrap()
            .retry(20)
            .unwrap();
        assert_ne!(next.job_id, input.job_id);
        assert_eq!(next.max_cycle, 17);
        assert_eq!(next.conv_tol, 1e-10);
        assert_eq!(next.document_revision, 20);
        assert_eq!(next.input_hash, input.input_hash);
        store.begin(&next, Some(&input.job_id)).unwrap();
        assert_eq!(
            store.page(None).unwrap().items[0].retry_of.as_deref(),
            Some(input.job_id.as_str())
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corruption_or_missing_files_never_restore_a_result() {
        for name in [
            "input.json",
            "result.json",
            "result.h5",
            "completion.json",
            "missing",
        ] {
            let root = root();
            let store = Catalog::open(&root).unwrap();
            let input = request();
            store.begin(&input, None).unwrap();
            let calculation = outputs(&root, &input);
            store.complete(&input, &calculation).unwrap();
            let directory = root.join("jobs").join(&input.job_id);
            if name == "missing" {
                fs::remove_file(directory.join("result.h5")).unwrap();
            } else {
                fs::write(directory.join(name), b"changed").unwrap();
            }
            assert!(store.restore(&input.job_id).is_err(), "{name}");
            assert_eq!(store.page(None).unwrap().items[0].status, "corrupt");
            assert!(store.retry_source(&input.job_id).is_ok());
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn cancellation_wins_over_late_outputs_and_survives_restart() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let input = request();
        store.begin(&input, None).unwrap();
        store.request_cancel(&input.job_id).unwrap();
        let calculation = outputs(&root, &input);
        assert_eq!(
            store.complete(&input, &calculation).unwrap_err(),
            "cancelled"
        );
        drop(store);
        let store = Catalog::open(&root).unwrap();
        assert_eq!(store.page(None).unwrap().items[0].status, "cancelled");
        assert!(store.restore(&input.job_id).is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn valid_receipt_recovers_completion_but_bare_outputs_do_not() {
        for receipt in [false, true] {
            let root = root();
            let store = Catalog::open(&root).unwrap();
            let input = request();
            store.begin(&input, None).unwrap();
            let calculation = outputs(&root, &input);
            if receipt {
                store.make_receipt(&input, &calculation).unwrap();
            }
            drop(store);
            let store = Catalog::open(&root).unwrap();
            assert_eq!(
                store.page(None).unwrap().items[0].status,
                if receipt { "succeeded" } else { "interrupted" }
            );
            assert_eq!(store.restore(&input.job_id).is_ok(), receipt);
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn legacy_records_import_once_with_explicit_baseline() {
        let root = root();
        let input = request();
        let directory = root.join("jobs").join(&input.job_id);
        fs::create_dir_all(&directory).unwrap();
        atomic_json(&directory.join("input.json"), &input).unwrap();
        outputs(&root, &input);
        atomic_json(
            &directory.join("status.json"),
            &JobSnapshot {
                id: input.job_id.clone(),
                status: "succeeded".into(),
                cycle: 2,
                error: None,
            },
        )
        .unwrap();
        let store = Catalog::open(&root).unwrap();
        assert!(store.restore(&input.job_id).is_ok());
        assert!(store.page(None).unwrap().items[0].legacy);
        drop(store);
        fs::write(directory.join("result.h5"), b"changed after migration").unwrap();
        let store = Catalog::open(&root).unwrap();
        assert!(store.restore(&input.job_id).is_err());
        assert_eq!(store.page(None).unwrap().items.len(), 1);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pagination_and_exclusive_lease_are_stable() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        assert!(matches!(Catalog::open(&root),Err(error) if error=="workspace_in_use"));
        for _ in 0..45 {
            let input = request();
            store.begin(&input, None).unwrap();
            store
                .fail(&input.job_id, "failed", Some("fixture"))
                .unwrap();
        }
        let first = store.page(None).unwrap();
        let second = store.page(first.next_cursor).unwrap();
        let third = store.page(second.next_cursor).unwrap();
        assert_eq!(
            [first.items.len(), second.items.len(), third.items.len()],
            [20, 20, 5]
        );
        assert!(third.next_cursor.is_none());
        assert!(first.items.last().unwrap().sequence > second.items[0].sequence);
        assert!(store.retry_source("../../workspace.json").is_err());
        drop(store);
        let store = Catalog::open(&root).unwrap();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn process_exit_leaves_a_recoverable_running_record() {
        const VARIABLE: &str = "REACTQM_CATALOG_CRASH_TEST";
        if let Some(root) = std::env::var_os(VARIABLE) {
            let root = PathBuf::from(root);
            let store = Catalog::open(&root).unwrap();
            store.begin(&request(), None).unwrap();
            store.enqueue(&request(), None).unwrap();
            store.enqueue(&request(), None).unwrap();
            std::process::exit(23); // Intentionally bypass destructors; OS must release the lease.
        }
        let root = root();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "catalog::tests::process_exit_leaves_a_recoverable_running_record",
                "--nocapture",
            ])
            .env(VARIABLE, &root)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(23));
        let store = Catalog::open(&root).unwrap();
        assert!(store
            .page(None)
            .unwrap()
            .items
            .iter()
            .any(|row| row.status == "interrupted"));
        let queue = store.queue_page(None).unwrap();
        assert!(queue.paused);
        assert_eq!(queue.waiting, 2);
        assert!(queue.active.is_none());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn saved_float_coordinates_keep_their_identity() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let input = JobRequest::new(
            crate::document::Document::from_xyz(
                "2\nH2\nH 17.301817144408927 0 0\nH 17.301817144408927 0 0.7400000000000001",
                0,
                0,
            )
            .unwrap(),
            1,
        )
        .unwrap();
        store.begin(&input, None).unwrap();
        assert_eq!(store.row(&input.job_id).unwrap().0.document, input.document);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reindexing_never_rebaselines_existing_receipts_or_uncancels_jobs() {
        for cancelled in [false, true] {
            let root = root();
            let store = Catalog::open(&root).unwrap();
            let input = request();
            store.begin(&input, None).unwrap();
            let result = outputs(&root, &input);
            if cancelled {
                store.make_receipt(&input, &result).unwrap();
                store.fail(&input.job_id, "cancelled", None).unwrap();
            } else {
                store.complete(&input, &result).unwrap();
            }
            drop(store);
            fs::remove_file(root.join("jobs.sqlite3")).unwrap();
            if !cancelled {
                fs::write(
                    root.join("jobs").join(&input.job_id).join("result.h5"),
                    b"\x89HDF\r\n\x1a\nchanged after completion",
                )
                .unwrap();
            }
            let store = Catalog::open(&root).unwrap();
            assert_eq!(
                store.page(None).unwrap().items[0].status,
                if cancelled { "cancelled" } else { "corrupt" }
            );
            assert!(store.restore(&input.job_id).is_err());
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn incomplete_artifacts_fail_without_stranding_a_running_record() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let input = request();
        store.begin(&input, None).unwrap();
        let result = outputs(&root, &input);
        fs::remove_file(root.join("jobs").join(&input.job_id).join("result.h5")).unwrap();
        assert!(store.complete(&input, &result).is_err());
        assert_eq!(store.page(None).unwrap().items[0].status, "failed");
        assert!(store.retry_source(&input.job_id).is_ok());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn queued_order_and_cancellation_survive_restart() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let inputs = [request(), request(), request()];
        for input in &inputs {
            store.enqueue(input, None).unwrap();
        }
        store.cancel_queued(&inputs[1].job_id).unwrap();
        store.set_queue_paused(false).unwrap();
        drop(store);
        let store = Catalog::open(&root).unwrap();
        let page = store.queue_page(None).unwrap();
        assert!(page.paused);
        assert_eq!(page.waiting, 2);
        assert_eq!(
            page.items.iter().map(|i| &i.id).collect::<Vec<_>>(),
            vec![&inputs[0].job_id, &inputs[2].job_id]
        );
        assert!(matches!(store.claim_next().unwrap(), Claim::Empty));
        store.set_queue_paused(false).unwrap();
        for input in [&inputs[0], &inputs[2]] {
            let Claim::Ready { request, .. } = store.claim_next().unwrap() else {
                panic!("missing FIFO claim")
            };
            assert_eq!(request.job_id, input.job_id);
            assert!(matches!(store.claim_next().unwrap(), Claim::Empty));
            store
                .fail(&request.job_id, "failed", Some("test-fixture"))
                .unwrap();
        }
        assert!(matches!(store.claim_next().unwrap(), Claim::Empty));
        assert_eq!(store.row(&inputs[1].job_id).unwrap().1, "cancelled");
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pause_keeps_active_job_and_blocks_the_next_claim() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let a = request();
        let b = request();
        store.enqueue(&a, None).unwrap();
        store.enqueue(&b, None).unwrap();
        store.set_queue_paused(false).unwrap();
        assert!(matches!(store.claim_next().unwrap(), Claim::Ready { .. }));
        store.set_queue_paused(true).unwrap();
        let page = store.queue_page(None).unwrap();
        assert_eq!(page.active.unwrap().id, a.job_id);
        assert_eq!(page.waiting, 1);
        store.fail(&a.job_id, "failed", None).unwrap();
        assert!(matches!(store.claim_next().unwrap(), Claim::Empty));
        store.set_queue_paused(false).unwrap();
        let Claim::Ready { request, .. } = store.claim_next().unwrap() else {
            panic!("resume failed")
        };
        assert_eq!(request.job_id, b.job_id);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn claim_cancel_races_never_cancel_a_different_job() {
        use std::sync::{Arc, Barrier};
        for _ in 0..12 {
            let root = root();
            let store = Arc::new(Catalog::open(&root).unwrap());
            let input = request();
            store.enqueue(&input, None).unwrap();
            store.set_queue_paused(false).unwrap();
            let barrier = Arc::new(Barrier::new(3));
            let claimant = {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.claim_next().unwrap()
                })
            };
            let canceller = {
                let store = store.clone();
                let barrier = barrier.clone();
                let id = input.job_id.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.cancel_queued(&id)
                })
            };
            barrier.wait();
            let claim = claimant.join().unwrap();
            let cancelled = canceller.join().unwrap();
            match claim {
                Claim::Ready { request, .. } => {
                    assert_eq!(request.job_id, input.job_id);
                    assert_eq!(cancelled.unwrap_err(), "job_not_queued");
                    assert_eq!(store.row(&input.job_id).unwrap().1, "running");
                }
                Claim::Empty => {
                    cancelled.unwrap();
                    assert_eq!(store.row(&input.job_id).unwrap().1, "cancelled");
                }
                Claim::Rejected { .. } => panic!("valid input rejected"),
            }
            drop(store);
            fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn concurrent_claims_have_a_single_winner() {
        use std::sync::Arc;
        let root = root();
        let store = Arc::new(Catalog::open(&root).unwrap());
        for _ in 0..3 {
            store.enqueue(&request(), None).unwrap();
        }
        store.set_queue_paused(false).unwrap();
        let handles = (0..8)
            .map(|_| {
                let store = store.clone();
                std::thread::spawn(move || store.claim_next().unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|claim| matches!(claim, Claim::Ready { .. }))
                .count(),
            1
        );
        assert_eq!(store.queue_page(None).unwrap().waiting, 2);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn damaged_waiting_input_is_rejected_before_launch_and_next_can_run() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        let a = request();
        let b = request();
        store.enqueue(&a, None).unwrap();
        store.enqueue(&b, None).unwrap();
        fs::write(
            root.join("jobs").join(&a.job_id).join("input.json"),
            b"changed",
        )
        .unwrap();
        store.set_queue_paused(false).unwrap();
        assert!(matches!(store.claim_next().unwrap(),Claim::Rejected { id,.. } if id==a.job_id));
        assert_eq!(store.row(&a.job_id).unwrap().1, "failed");
        assert!(
            matches!(store.claim_next().unwrap(),Claim::Ready { request,.. } if request.job_id==b.job_id)
        );
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn queue_pagination_and_bound_are_enforced() {
        let root = root();
        let store = Catalog::open(&root).unwrap();
        for _ in 0..100 {
            store.enqueue(&request(), None).unwrap();
        }
        let overflow = request();
        assert_eq!(store.enqueue(&overflow, None).unwrap_err(), "queue_full");
        assert!(!root.join("jobs").join(overflow.job_id).exists());
        let first = store.queue_page(None).unwrap();
        let second = store.queue_page(first.next_cursor).unwrap();
        assert_eq!(first.items.len(), 20);
        assert_eq!(second.items.len(), 20);
        assert!(first.items[19].sequence < second.items[0].sequence);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
