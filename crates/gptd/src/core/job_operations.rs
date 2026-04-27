use super::CoreService;
use super::support::lock_cancel_registry;
use anyhow::{Context, Result};
use chrono::Utc;
use shared_protocol::{
    AckEvent, CancelJobCommand, JobDetail, JobList, JobStatus, RequestHistory, RequestListPage,
    RequestSummary,
};

impl CoreService {
    pub fn list_jobs(&self) -> Result<JobList> {
        self.open_db()?.list_jobs()
    }

    pub fn job_detail(&self, job_id: &str) -> Result<JobDetail> {
        self.open_db()?
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))
    }

    pub fn record_request_summary(&self, summary: &RequestSummary) -> Result<()> {
        self.open_db()?.upsert_request_summary(summary)
    }

    pub fn request_summary(&self, request_id: &str) -> Result<RequestSummary> {
        self.open_db()?
            .request_summary(request_id)?
            .with_context(|| format!("request {request_id} was not found"))
    }

    pub fn list_request_summaries(&self, limit: usize) -> Result<RequestHistory> {
        self.open_db()?.list_request_summaries(limit)
    }

    pub fn list_request_page(
        &self,
        status: Option<JobStatus>,
        limit: usize,
        offset: usize,
    ) -> Result<RequestListPage> {
        self.open_db()?.list_request_page(status, limit, offset)
    }

    pub fn cancel_job(&self, command: CancelJobCommand) -> Result<AckEvent> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(&command.job_id)?
            .with_context(|| format!("job {} was not found", command.job_id))?;
        if matches!(
            detail.status,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed
        ) {
            Ok(AckEvent {
                accepted: false,
                message: format!("job {} is already finished", command.job_id),
            })
        } else {
            db.set_job_cancel_requested(&command.job_id, true)?;
            lock_cancel_registry(&self.cancel_registry)?.insert(command.job_id.clone());
            Ok(AckEvent {
                accepted: true,
                message: format!("cancellation requested for {}", command.job_id),
            })
        }
    }

    pub fn job_cancel_requested(&self, job_id: &str) -> Result<bool> {
        self.open_db()?.job_cancel_requested(job_id)
    }

    pub fn fail_job(&self, job_id: &str, message: &str) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        db.insert_job(
            job_id,
            JobStatus::Failed,
            Some(message),
            detail.conversation_id.as_deref(),
            detail.created_at,
            Utc::now(),
        )?;
        db.set_job_cancel_requested(job_id, false)?;
        Ok(())
    }

    pub fn mark_job_cancelled(&self, job_id: &str, message: &str) -> Result<()> {
        self.cancel_job_with_detail(job_id, None, message)
    }

    pub fn mark_job_interrupted(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        let interrupted_at = Utc::now();
        db.insert_job(
            job_id,
            JobStatus::Interrupted,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            interrupted_at,
        )?;
        db.record_job_interruption(
            job_id,
            interrupted_at,
            message,
            detail.current_url.as_deref(),
        )?;
        Ok(())
    }

    pub fn mark_job_recovered(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        let recovered_at = Utc::now();
        db.insert_job(
            job_id,
            JobStatus::Recovered,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            recovered_at,
        )?;
        db.record_job_recovery(job_id, recovered_at, message, detail.current_url.as_deref())?;
        Ok(())
    }

    pub(super) fn cancel_job_with_detail(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        db.insert_job(
            job_id,
            JobStatus::Cancelled,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            Utc::now(),
        )?;
        self.clear_cancel_requested(&db, job_id)
    }
}
