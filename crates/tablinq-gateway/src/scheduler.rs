use anyhow::{Result, bail};
use gptd::AppConfig;
use serde::Serialize;
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct SchedulerSnapshot {
    pub(crate) running: u32,
    pub(crate) queued: u32,
    pub(crate) max_concurrent_upstream_jobs: u32,
    pub(crate) queue_length: u32,
    pub(crate) queue_timeout_ms: u64,
    pub(crate) per_client_limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Queued,
    Running,
}

#[derive(Debug, Clone)]
struct TrackedRequest {
    job_id: String,
    client_id: String,
    state: RequestState,
    cancelled: bool,
}

#[derive(Debug, Default)]
struct SchedulerState {
    running: u32,
    queue: VecDeque<String>,
    requests: BTreeMap<String, TrackedRequest>,
}

#[derive(Debug)]
pub(crate) struct GatewayScheduler {
    max_concurrent_upstream_jobs: u32,
    queue_length: u32,
    queue_timeout: Duration,
    per_client_limit: u32,
    inner: Mutex<SchedulerState>,
    changed: Condvar,
}

#[derive(Debug)]
pub(crate) struct SchedulerLease<'a> {
    scheduler: &'a GatewayScheduler,
    request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelDisposition {
    Queued,
    Running,
}

impl GatewayScheduler {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self {
            max_concurrent_upstream_jobs: config.gateway_max_concurrent_upstream_jobs(),
            queue_length: config.gateway_queue_length(),
            queue_timeout: Duration::from_millis(config.gateway_queue_timeout_ms()),
            per_client_limit: config.gateway_per_client_limit(),
            inner: Mutex::new(SchedulerState::default()),
            changed: Condvar::new(),
        }
    }

    pub(crate) fn acquire<'a>(
        &'a self,
        request_id: &str,
        job_id: &str,
        client_id: &str,
    ) -> Result<SchedulerLease<'a>> {
        let mut state = lock_scheduler(&self.inner)?;
        if client_request_count(&state, client_id) >= self.per_client_limit {
            bail!("per-client queue limit reached");
        }
        if state.running < self.max_concurrent_upstream_jobs && state.queue.is_empty() {
            state.running += 1;
            state.requests.insert(
                request_id.to_owned(),
                TrackedRequest {
                    job_id: job_id.to_owned(),
                    client_id: client_id.to_owned(),
                    state: RequestState::Running,
                    cancelled: false,
                },
            );
            return Ok(SchedulerLease {
                scheduler: self,
                request_id: request_id.to_owned(),
            });
        }

        if state.queue.len() as u32 >= self.queue_length {
            bail!("queue full");
        }
        state.queue.push_back(request_id.to_owned());
        state.requests.insert(
            request_id.to_owned(),
            TrackedRequest {
                job_id: job_id.to_owned(),
                client_id: client_id.to_owned(),
                state: RequestState::Queued,
                cancelled: false,
            },
        );

        let deadline = Instant::now() + self.queue_timeout;
        loop {
            if let Some(entry) = state.requests.get(request_id)
                && entry.cancelled
            {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("request cancelled while queued");
            }

            let ready = state.running < self.max_concurrent_upstream_jobs
                && state.queue.front().map(String::as_str) == Some(request_id);
            if ready {
                state.queue.pop_front();
                state.running += 1;
                if let Some(entry) = state.requests.get_mut(request_id) {
                    entry.state = RequestState::Running;
                }
                return Ok(SchedulerLease {
                    scheduler: self,
                    request_id: request_id.to_owned(),
                });
            }

            let now = Instant::now();
            if now >= deadline {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("queue timeout exceeded");
            }

            let timeout = deadline.saturating_duration_since(now);
            let (next_state, wait_result) = self
                .changed
                .wait_timeout(state, timeout)
                .map_err(|_| anyhow::anyhow!("gateway scheduler lock poisoned"))?;
            state = next_state;
            if wait_result.timed_out() {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("queue timeout exceeded");
            }
        }
    }

    pub(crate) fn cancel(&self, request_id: &str) -> Result<Option<(String, CancelDisposition)>> {
        let mut state = lock_scheduler(&self.inner)?;
        let Some(entry) = state.requests.get_mut(request_id) else {
            return Ok(None);
        };
        entry.cancelled = true;
        let disposition = match entry.state {
            RequestState::Queued => CancelDisposition::Queued,
            RequestState::Running => CancelDisposition::Running,
        };
        let job_id = entry.job_id.clone();
        self.changed.notify_all();
        Ok(Some((job_id, disposition)))
    }

    pub(crate) fn snapshot(&self) -> Result<SchedulerSnapshot> {
        let state = lock_scheduler(&self.inner)?;
        Ok(SchedulerSnapshot {
            running: state.running,
            queued: state.queue.len() as u32,
            max_concurrent_upstream_jobs: self.max_concurrent_upstream_jobs,
            queue_length: self.queue_length,
            queue_timeout_ms: self.queue_timeout.as_millis() as u64,
            per_client_limit: self.per_client_limit,
        })
    }
}

impl Drop for SchedulerLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scheduler.inner.lock() {
            if state.requests.remove(&self.request_id).is_some() && state.running > 0 {
                state.running -= 1;
            }
            self.scheduler.changed.notify_all();
        }
    }
}

fn lock_scheduler<'a>(
    mutex: &'a Mutex<SchedulerState>,
) -> Result<std::sync::MutexGuard<'a, SchedulerState>> {
    mutex
        .lock()
        .map_err(|_| anyhow::anyhow!("gateway scheduler lock poisoned"))
}

fn remove_queued_request(state: &mut SchedulerState, request_id: &str) {
    state.requests.remove(request_id);
    if let Some(index) = state.queue.iter().position(|value| value == request_id) {
        state.queue.remove(index);
    }
}

fn client_request_count(state: &SchedulerState, client_id: &str) -> u32 {
    state
        .requests
        .values()
        .filter(|entry| entry.client_id == client_id)
        .count() as u32
}
