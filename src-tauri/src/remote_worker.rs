//! Remote single-point execution via the loopback Worker client.
use crate::{
    jobs::{JobRequest, OwnedChild, WorkerConfig},
    project::Calculation,
};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn client(config: &WorkerConfig) -> Command {
    let mut command = Command::new(&config.python);
    command
        .args(["-m", "reactqm_worker"])
        .current_dir(&config.root)
        .env("PYTHONPATH", config.root.join("src"))
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
}

pub(crate) fn cancel_remote(config: &WorkerConfig, id: &str, port: u16) -> Result<(), String> {
    let mut child = OwnedChild(
        client(config)
            .args([
                "remote-cancel",
                "--port",
                &port.to_string(),
                "--job-id",
                id,
                "--wait",
            ])
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "remote_cancel_unconfirmed")?,
    );
    let deadline = Instant::now() + Duration::from_secs(35);
    while Instant::now() < deadline {
        if let Some(status) = child
            .0
            .try_wait()
            .map_err(|_| "remote_cancel_unconfirmed")?
        {
            return match status.code() {
                Some(0) => Ok(()),
                Some(3) => Err("remote_cancel_too_late".into()),
                Some(4) => Err("remote_cancel_missing".into()),
                _ => Err("remote_cancel_unconfirmed".into()),
            };
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err("remote_cancel_unconfirmed".into())
}

pub(crate) fn execute(
    request: &JobRequest,
    directory: &Path,
    config: &WorkerConfig,
    cancel: &AtomicBool,
    port: u16,
    mut progress: impl FnMut(u32),
) -> Result<Calculation, String> {
    if cancel.load(Ordering::SeqCst) {
        cancel_remote(config, &request.job_id, port)?;
        return Err("cancelled".into());
    }
    let port_arg = port.to_string();
    // A client orphaned by a forced desktop exit may still be downloading.
    let stage = directory.join(format!("remote-download-{}", uuid::Uuid::new_v4()));
    let log = File::create(directory.join("worker.log")).map_err(|_| "storage")?;
    let mut child = OwnedChild(
        client(config)
            .args([
                "remote-run",
                "--phase-stream",
                "--port",
                &port_arg,
                "--input",
            ])
            .stdout(Stdio::piped())
            .arg(directory.join("input.json"))
            .arg("--output-dir")
            .arg(&stage)
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|_| "worker_missing")?,
    );
    let mut stream = child.0.stdout.take().ok_or("worker_protocol")?;
    let latest = Arc::new(AtomicU8::new(0));
    let producer = latest.clone();
    thread::spawn(move || {
        let mut phase = [0];
        while stream.read_exact(&mut phase).is_ok() {
            if (1..=5).contains(&phase[0]) {
                producer.store(phase[0], Ordering::Relaxed);
            }
        }
    });
    let mut shown = 0;
    let deadline = Instant::now() + Duration::from_secs(360);
    loop {
        if cancel.load(Ordering::SeqCst) {
            // Stop new submissions before cancelling the immutable remote job ID.
            drop(child);
            cancel_remote(config, &request.job_id, port)?;
            return Err("cancelled".into());
        }
        let phase = latest.load(Ordering::Relaxed);
        if phase != shown {
            shown = phase;
            progress(u32::from(phase));
        }
        if Instant::now() >= deadline {
            return Err("timed_out".into());
        }
        if let Some(status) = child.0.try_wait().map_err(|_| "remote_worker_failed")? {
            if !status.success() {
                return Err("remote_worker_failed".into());
            }
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    let read_json = |name| -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        File::open(stage.join(name))
            .map_err(|_| "artifact_invalid")?
            .take(65_537)
            .read_to_end(&mut bytes)
            .map_err(|_| "artifact_invalid")?;
        if bytes.len() > 65_536 {
            return Err("artifact_invalid".into());
        }
        Ok(bytes)
    };
    #[derive(serde::Deserialize)]
    struct Manifest {
        schema_version: u32,
        job_id: String,
        status: String,
        artifacts: Vec<Artifact>,
    }
    #[derive(serde::Deserialize)]
    struct Artifact {
        name: String,
        bytes: u64,
        sha256: String,
    }
    let manifest: Manifest = serde_json::from_slice(&read_json("remote-manifest.json")?)
        .map_err(|_| "artifact_invalid")?;
    if manifest.schema_version != 1
        || manifest.job_id != request.job_id
        || manifest.status != "succeeded"
        || manifest.artifacts.len() != 3
    {
        return Err("artifact_invalid".into());
    }
    for name in ["input.json", "result.json", "result.h5"] {
        let artifact = manifest
            .artifacts
            .iter()
            .find(|item| item.name == name)
            .ok_or("artifact_invalid")?;
        if crate::catalog::digest(&stage.join(name))? != (artifact.bytes, artifact.sha256.clone()) {
            return Err("artifact_invalid".into());
        }
    }
    let input: JobRequest =
        serde_json::from_slice(&read_json("input.json")?).map_err(|_| "artifact_invalid")?;
    if serde_json::to_vec(&input).map_err(|_| "artifact_invalid")?
        != serde_json::to_vec(request).map_err(|_| "artifact_invalid")?
    {
        return Err("artifact_invalid".into());
    }
    let result: Calculation =
        serde_json::from_slice(&read_json("result.json")?).map_err(|_| "artifact_invalid")?;
    request.validate_result(&result)?;
    for name in ["result.json", "result.h5"] {
        let file = File::options()
            .write(true)
            .open(stage.join(name))
            .map_err(|_| "storage")?;
        file.sync_all().map_err(|_| "storage")?;
        fs::rename(stage.join(name), directory.join(name)).map_err(|_| "storage")?;
    }
    let _ = fs::remove_dir_all(stage);
    Ok(result)
}
