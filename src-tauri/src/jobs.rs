//! One bounded local job at a time; process ownership never moves into a window.
use crate::{
    document::Document,
    project::{atomic_json, Calculation},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub schema_version: u32,
    pub job_id: String,
    pub document_revision: u64,
    pub input_hash: String,
    pub document: Document,
    pub method: String,
    pub basis: String,
    pub max_cycle: u32,
    pub conv_tol: f64,
}
impl JobRequest {
    pub fn validate(&self) -> Result<(), String> {
        self.document.validate()?;
        if self.document.spin != 0 {
            return Err("rhf_spin".into());
        }
        if self.document.atoms.iter().any(|a| a.atomic_number > 10) {
            return Err("rhf_elements".into());
        }
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.job_id)
                .map_err(|_| "job_invalid")?
                .to_string()
                != self.job_id
            || self.input_hash != self.document.fingerprint()
            || self.method != "RHF"
            || self.basis != "sto-3g"
            || !(1..=50).contains(&self.max_cycle)
            || !(1e-12..=1e-6).contains(&self.conv_tol)
        {
            return Err("job_invalid".into());
        }
        Ok(())
    }
    pub fn retry(&self, revision: u64) -> Result<Self, String> {
        self.validate()?;
        let mut next = self.clone();
        next.job_id = uuid::Uuid::new_v4().to_string();
        next.document_revision = revision;
        Ok(next)
    }
    pub fn new(document: Document, document_revision: u64) -> Result<Self, String> {
        document.validate()?;
        if document.spin != 0 {
            return Err("rhf_spin".into());
        }
        // First validated engine scope: H–Ne. Expand only with basis/engine fixtures.
        if document.atoms.iter().any(|a| a.atomic_number > 10) {
            return Err("rhf_elements".into());
        }
        Ok(Self {
            schema_version: 1,
            job_id: uuid::Uuid::new_v4().to_string(),
            document_revision,
            input_hash: document.fingerprint(),
            document,
            method: "RHF".into(),
            basis: "sto-3g".into(),
            max_cycle: 50,
            conv_tol: 1e-9,
        })
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkerEvent {
    Progress {
        schema_version: u32,
        job_id: String,
        cycle: u32,
    },
    Result {
        schema_version: u32,
        calculation: Calculation,
    },
    Error {
        schema_version: u32,
        job_id: String,
        code: String,
    },
}

pub struct WorkerConfig {
    pub python: PathBuf,
    pub root: PathBuf,
}
impl WorkerConfig {
    pub fn development() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../worker");
        let python = std::env::var_os("REACTQM_PYTHON")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                root.join(if cfg!(windows) {
                    ".venv/Scripts/python.exe"
                } else {
                    ".venv/bin/python"
                })
            });
        Self { python, root }
    }
}

pub(crate) struct OwnedChild(pub(crate) Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // The Unix group is created exclusively for this job, including future subprocesses.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn execute(
    request: &JobRequest,
    directory: &Path,
    config: &WorkerConfig,
    cancel: Arc<AtomicBool>,
    timeout: Duration,
    mut progress: impl FnMut(u32),
) -> Result<Calculation, String> {
    request.validate()?;
    fs::create_dir_all(directory).map_err(|e| format!("storage: {e}"))?;
    atomic_json(&directory.join("input.json"), request)?;
    if !config.python.is_file() {
        return Err("worker_missing".into());
    }
    let log = File::create(directory.join("worker.log")).map_err(|e| format!("storage: {e}"))?;
    let mut command = Command::new(&config.python);
    command
        .args(["-m", "reactqm_worker", "calculate", "--output-dir"])
        .arg(directory)
        .current_dir(&config.root)
        .env("PYTHONPATH", config.root.join("src"))
        .env("OMP_NUM_THREADS", "1")
        .env("OPENBLAS_NUM_THREADS", "1")
        .env("MKL_NUM_THREADS", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = OwnedChild(command.spawn().map_err(|e| format!("worker_start: {e}"))?);
    let mut input = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    input.push(b'\n');
    child
        .0
        .stdin
        .take()
        .ok_or("worker_protocol")?
        .write_all(&input)
        .map_err(|e| format!("worker_protocol: {e}"))?;
    let stdout = child.0.stdout.take().ok_or("worker_protocol")?;
    let (sender, receiver) = mpsc::sync_channel(16);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            let result = reader.by_ref().take(8193).read_until(b'\n', &mut bytes);
            if matches!(result, Ok(0)) {
                break;
            }
            let event = match result {
                Ok(_) if bytes.len() <= 8192 => serde_json::from_slice::<WorkerEvent>(&bytes)
                    .map_err(|_| "worker_protocol".to_string()),
                _ => Err("worker_protocol".to_string()),
            };
            let invalid = event.is_err();
            if sender.send(event).is_err() || invalid {
                break;
            }
        }
    });
    let started = Instant::now();
    let mut result = None;
    let mut worker_error = None;
    let mut exited = None;
    let mut disconnected = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        if started.elapsed() > timeout {
            return Err("timed_out".into());
        }
        // Bounded draining prevents a noisy child from starving timeout/cancellation.
        for _ in 0..32 {
            match receiver.try_recv() {
                Ok(event) => match event? {
                    WorkerEvent::Progress {
                        schema_version: 1,
                        job_id,
                        cycle,
                    } if job_id == request.job_id && cycle <= request.max_cycle + 1 => {
                        progress(cycle)
                    }
                    WorkerEvent::Result {
                        schema_version: 1,
                        calculation,
                    } if calculation.job_id == request.job_id && result.is_none() => {
                        calculation.validate(&request.document)?;
                        result = Some(calculation);
                    }
                    WorkerEvent::Error {
                        schema_version: 1,
                        job_id,
                        code,
                    } if job_id == request.job_id => {
                        worker_error = Some(if code == "not_converged" {
                            code
                        } else {
                            "calculation_failed".into()
                        });
                    }
                    _ => return Err("worker_protocol".into()),
                },
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if exited.is_none() {
            exited = child
                .0
                .try_wait()
                .map_err(|e| format!("worker_exit: {e}"))?;
        }
        if let Some(status) = exited {
            if disconnected {
                if let Some(error) = worker_error {
                    return Err(error);
                }
                if !status.success() {
                    return Err("worker_exit".into());
                }
                return result.ok_or_else(|| "worker_protocol".into());
            }
        }
        std::thread::sleep(Duration::from_millis(40));
    }
}
