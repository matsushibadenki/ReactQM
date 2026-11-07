//! One application-owned FIFO executor. A gate serializes claim / pause / shutdown.
use crate::{
    catalog::{Catalog, CatalogState, Claim},
    jobs::{self, JobRequest, WorkerConfig},
    project::Calculation,
    state::{ActiveJob, AppState, JobSnapshot},
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub struct Gate {
    pub stopping: bool,
    notified: bool,
}
#[derive(Default)]
pub struct Scheduler {
    pub gate: Mutex<Gate>,
    changed: Condvar,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl Scheduler {
    pub fn wake(&self) {
        if let Ok(mut gate) = self.gate.lock() {
            gate.notified = true;
            self.changed.notify_one();
        }
    }
    pub fn shutdown(&self) {
        if let Ok(mut gate) = self.gate.lock() {
            gate.stopping = true;
            gate.notified = true;
            self.changed.notify_one();
        }
    }
    pub fn join(&self) {
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(handle) = worker.take() {
                if handle.thread().id() != std::thread::current().id() {
                    let _ = handle.join();
                }
            }
        }
    }
    pub fn start(app: tauri::AppHandle) {
        let owner = app.clone();
        let handle = std::thread::spawn(move || loop {
            let runner = app.state::<Scheduler>();
            let mut gate = match runner.gate.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            loop {
                if gate.stopping {
                    return;
                }
                if gate.notified {
                    gate.notified = false;
                    break;
                }
                gate = match runner.changed.wait(gate) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
            let claimed = (|| {
                let state = app.state::<AppState>();
                let mut workspace = state.0.lock().map_err(|e| e.to_string())?;
                if workspace.active_job.is_some() {
                    return Ok(Claim::Empty);
                }
                let claim = app.state::<CatalogState>().get()?.claim_next()?;
                if let Claim::Ready { request, .. } = &claim {
                    let cancel = Arc::new(AtomicBool::new(false));
                    workspace.active_job = Some(ActiveJob {
                        id: request.job_id.clone(),
                        cancel,
                    });
                    workspace.job = Some(JobSnapshot {
                        id: request.job_id.clone(),
                        status: "running".into(),
                        cycle: 0,
                        error: None,
                    });
                    workspace.revision += 1;
                    let _ = app.emit("workspace-changed", workspace.snapshot());
                }
                Ok::<_, String>(claim)
            })();
            drop(gate);
            match claimed {
                Ok(Claim::Empty) => {}
                Ok(Claim::Rejected { id, .. }) => {
                    if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                        workspace.result_targets.remove(&id);
                    }
                    let _ = app.emit("job-history-changed", ());
                    runner.wake();
                }
                Err(error) => {
                    let _ = app
                        .state::<CatalogState>()
                        .get()
                        .and_then(|store| store.set_queue_paused(true));
                    let _ = app.emit("queue-error", error);
                    let _ = app.emit("job-history-changed", ());
                }
                Ok(Claim::Ready { request, directory }) => {
                    let _ = app.emit("job-history-changed", ());
                    let cancel = {
                        let state = app.state::<AppState>();
                        let workspace = state.0.lock().unwrap();
                        workspace.active_job.as_ref().unwrap().cancel.clone()
                    };
                    let config = WorkerConfig::development();
                    let result = run_claimed(
                        app.state::<CatalogState>().get().unwrap(),
                        &request,
                        &directory,
                        &config,
                        cancel,
                        |cycle| {
                            if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                                if let Some(job) = &mut workspace.job {
                                    if job.id == request.job_id {
                                        job.cycle = cycle;
                                    }
                                }
                                workspace.revision += 1;
                                let _ = app.emit("workspace-changed", workspace.snapshot());
                            }
                        },
                    );
                    let (status, error, calculation) = match result {
                        Ok(calculation) => ("succeeded", None, Some(calculation)),
                        Err(error) if error == "cancelled" => ("cancelled", None, None),
                        Err(error) if error == "timed_out" => ("timed_out", Some(error), None),
                        Err(error) => ("failed", Some(error), None),
                    };
                    if let Ok(mut workspace) = app.state::<AppState>().0.lock() {
                        workspace.complete_target(&request.job_id, calculation);
                        if let Some(job) = &mut workspace.job {
                            job.status = status.into();
                            job.error = error;
                        }
                        workspace.active_job = None;
                        workspace.revision += 1;
                        let _ = app.emit("workspace-changed", workspace.snapshot());
                        let exit = workspace.exit_after_job;
                        drop(workspace);
                        let _ = app.emit("job-history-changed", ());
                        if exit {
                            app.exit(0);
                            return;
                        }
                    }
                    runner.wake();
                }
            }
        });
        *owner.state::<Scheduler>().worker.lock().unwrap() = Some(handle);
    }
}
/// Shared by the application and real-process queue probes. The caller already owns a claim.
pub fn run_claimed(
    store: &Catalog,
    request: &JobRequest,
    directory: &Path,
    config: &WorkerConfig,
    cancel: Arc<AtomicBool>,
    progress: impl FnMut(u32),
) -> Result<Calculation, String> {
    let result = jobs::execute(
        request,
        directory,
        config,
        cancel.clone(),
        Duration::from_secs(120),
        progress,
    );
    let result = if cancel.load(Ordering::SeqCst) {
        Err("cancelled".into())
    } else {
        result
    };
    match result {
        Ok(calculation) => match store.complete(request, &calculation) {
            Ok(()) => Ok(calculation),
            Err(error) if error == "cancelled" => {
                store.fail(&request.job_id, "cancelled", None)?;
                Err(error)
            }
            Err(error) => {
                // Storage failure must not cause subsequent queued work to run unattended.
                store.set_queue_paused(true)?;
                Err(error)
            }
        },
        Err(error) => {
            let status = match error.as_str() {
                "cancelled" => "cancelled",
                "timed_out" => "timed_out",
                _ => "failed",
            };
            if let Err(storage_error) = store.fail(
                &request.job_id,
                status,
                if status == "cancelled" {
                    None
                } else {
                    Some(&error)
                },
            ) {
                let _ = store.set_queue_paused(true);
                return Err(storage_error);
            }
            Err(error)
        }
    }
}
