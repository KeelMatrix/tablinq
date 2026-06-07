use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use shared_protocol::NativeUploadRateLimitDiagnostic;

pub(crate) struct NativeUploadBudgetRequest<'a> {
    pub(crate) scope_key: &'a str,
    pub(crate) configured_limit: u32,
    pub(crate) window_minutes: u32,
    pub(crate) attempted_upload_cost: u32,
    pub(crate) now: DateTime<Utc>,
}

pub(crate) enum NativeUploadBudgetReservation {
    Charged,
    Exhausted(NativeUploadRateLimitDiagnostic),
    NoUpload,
}

pub(super) fn reserve(
    conn: &mut Connection,
    request: NativeUploadBudgetRequest<'_>,
) -> Result<NativeUploadBudgetReservation> {
    if request.attempted_upload_cost == 0 {
        return Ok(NativeUploadBudgetReservation::NoUpload);
    }

    let window_start = request.now - Duration::minutes(i64::from(request.window_minutes));
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "DELETE FROM native_upload_attempts
         WHERE scope_key = ?1 AND charged_at <= ?2",
        params![request.scope_key, window_start.to_rfc3339()],
    )
    .context("prune expired native upload attempts")?;

    let uploads_currently_charged = current_charge_count(&tx, request.scope_key)?;
    let next_release_at = next_release_at(&tx, request.scope_key, request.window_minutes)?
        .unwrap_or_else(|| request.now + Duration::minutes(i64::from(request.window_minutes)));

    if uploads_currently_charged.saturating_add(request.attempted_upload_cost)
        >= request.configured_limit
    {
        tx.commit()?;
        return Ok(NativeUploadBudgetReservation::Exhausted(
            NativeUploadRateLimitDiagnostic {
                configured_limit: request.configured_limit,
                window_minutes: request.window_minutes,
                uploads_currently_charged,
                attempted_upload_cost: request.attempted_upload_cost,
                remaining_seconds_until_release: seconds_until(request.now, next_release_at),
                reset_at: next_release_at,
            },
        ));
    }

    tx.execute(
        "INSERT INTO native_upload_attempts
             (scope_key, upload_count, charged_at)
         VALUES (?1, ?2, ?3)",
        params![
            request.scope_key,
            i64::from(request.attempted_upload_cost),
            request.now.to_rfc3339()
        ],
    )
    .context("record native upload attempt")?;
    tx.commit()?;

    Ok(NativeUploadBudgetReservation::Charged)
}

pub(super) fn reset(conn: &mut Connection, scope_key: &str) -> Result<u64> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let deleted = tx
        .execute(
            "DELETE FROM native_upload_attempts WHERE scope_key = ?1",
            [scope_key],
        )
        .context("reset native upload attempts")?;
    tx.commit()?;
    Ok(deleted as u64)
}

#[cfg(test)]
pub(super) fn current_charged_uploads(
    conn: &Connection,
    scope_key: &str,
    window_minutes: u32,
    now: DateTime<Utc>,
) -> Result<u32> {
    let window_start = now - Duration::minutes(i64::from(window_minutes));
    let count = conn
        .query_row(
            "SELECT COALESCE(SUM(upload_count), 0)
             FROM native_upload_attempts
             WHERE scope_key = ?1 AND charged_at > ?2",
            params![scope_key, window_start.to_rfc3339()],
            |row| row.get::<_, i64>(0),
        )
        .context("count native upload attempts")?;
    Ok(count.max(0) as u32)
}

fn current_charge_count(tx: &rusqlite::Transaction<'_>, scope_key: &str) -> Result<u32> {
    let count = tx
        .query_row(
            "SELECT COALESCE(SUM(upload_count), 0)
             FROM native_upload_attempts
             WHERE scope_key = ?1",
            [scope_key],
            |row| row.get::<_, i64>(0),
        )
        .context("count native upload attempts")?;
    Ok(count.max(0) as u32)
}

fn next_release_at(
    tx: &rusqlite::Transaction<'_>,
    scope_key: &str,
    window_minutes: u32,
) -> Result<Option<DateTime<Utc>>> {
    let charged_at = tx
        .query_row(
            "SELECT charged_at
             FROM native_upload_attempts
             WHERE scope_key = ?1
             ORDER BY charged_at ASC
             LIMIT 1",
            [scope_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("read next native upload release")?;
    charged_at
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| {
                    parsed.with_timezone(&Utc) + Duration::minutes(i64::from(window_minutes))
                })
                .with_context(|| format!("parse native upload charged_at {value}"))
        })
        .transpose()
}

fn seconds_until(now: DateTime<Utc>, reset_at: DateTime<Utc>) -> u64 {
    (reset_at - now).num_seconds().max(0) as u64
}
