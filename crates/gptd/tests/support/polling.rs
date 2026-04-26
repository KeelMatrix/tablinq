use gptd::send_request;
use shared_protocol::{DaemonRequest, DaemonResponse, JobStatus, build_state_paths};
use std::path::Path;
use std::thread;
use std::time::{Duration as StdDuration, Instant};

pub fn wait_for_job_id(state_root: &Path) -> String {
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        let response = send_request(
            &build_state_paths(state_root).ipc_endpoint,
            &DaemonRequest::JobsList,
        )
        .unwrap();
        if let DaemonResponse::Jobs(list) = response
            && let Some(job) = list.jobs.first()
        {
            return job.job_id.clone();
        }
        thread::sleep(StdDuration::from_millis(50));
    }

    panic!("job id was not created in time");
}

pub fn wait_for_job_status(state_root: &Path, job_id: &str, expected: JobStatus) {
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        let response = send_request(
            &build_state_paths(state_root).ipc_endpoint,
            &DaemonRequest::JobsShow(job_id.to_owned()),
        )
        .unwrap();
        if let DaemonResponse::Job(job) = response
            && job.status == expected
        {
            return;
        }
        thread::sleep(StdDuration::from_millis(50));
    }

    panic!("job {job_id} did not reach status {expected:?} in time");
}
