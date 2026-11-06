use reactqm_lib::{
    document::Document,
    jobs::{execute, JobRequest, WorkerConfig},
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

fn run_failure(mode: &str, expected: &str, cancel_after: Option<Duration>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let directory = std::env::temp_dir().join(format!("reactqm-test-{}", uuid::Uuid::new_v4()));
    let document = Document::from_xyz(&format!("2\n{mode}\nH 0 0 0\nH 0 0 .74"), 0, 0).unwrap();
    let request = JobRequest::new(document, 1).unwrap();
    let config = WorkerConfig {
        python: root.join("../worker/.venv/bin/python"),
        root: root.join("tests/support"),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let handle = cancel_after.map(|delay| {
        let signal = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            signal.store(true, Ordering::SeqCst);
        })
    });
    let started = Instant::now();
    let result = execute(
        &request,
        &directory,
        &config,
        cancel,
        Duration::from_millis(800),
        |_| {},
    );
    assert_eq!(result.unwrap_err(), expected);
    assert!(started.elapsed() < Duration::from_secs(4));
    if let Some(handle) = handle {
        handle.join().unwrap();
    }
    #[cfg(unix)]
    if let Ok(pid) = std::fs::read_to_string(directory.join("pid")) {
        // execute must reap the child before returning, even after a protocol error.
        assert_eq!(unsafe { libc::kill(pid.parse::<i32>().unwrap(), 0) }, -1);
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn worker_failures_are_classified_and_reaped() {
    for (mode, expected) in [
        ("exit", "worker_exit"),
        ("invalid", "worker_protocol"),
        ("oversized", "worker_protocol"),
        ("foreign", "worker_protocol"),
        ("nonconverged", "not_converged"),
        ("stall", "timed_out"),
        ("flood", "timed_out"),
    ] {
        run_failure(mode, expected, None);
    }
}
#[test]
fn cancellation_reaps_the_child() {
    run_failure("stall", "cancelled", Some(Duration::from_millis(200)));
}
