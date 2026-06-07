use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use shared_protocol::{ChatGptMessageRateLimitDiagnostic, RateLimitBucket};

pub(crate) struct ChatGptMessageBudgetRequest<'a> {
    pub(crate) scope_key: &'a str,
    pub(crate) bucket: RateLimitBucket,
    pub(crate) configured_limit: u32,
    pub(crate) window_minutes: u32,
    pub(crate) attempted_message_cost: u32,
    pub(crate) now: DateTime<Utc>,
}

pub(crate) enum ChatGptMessageBudgetReservation {
    Charged,
    Exhausted(ChatGptMessageRateLimitDiagnostic),
}

pub(super) fn reserve(
    conn: &mut Connection,
    request: ChatGptMessageBudgetRequest<'_>,
) -> Result<ChatGptMessageBudgetReservation> {
    let window_start = request.now - Duration::minutes(i64::from(request.window_minutes));
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        "DELETE FROM chatgpt_message_attempts
         WHERE scope_key = ?1 AND charged_at <= ?2",
        params![request.scope_key, window_start.to_rfc3339()],
    )
    .context("prune expired ChatGPT message attempts")?;

    let messages_currently_charged = current_charge_count(&tx, request.scope_key)?;
    let next_release_at = next_release_at(&tx, request.scope_key, request.window_minutes)?
        .unwrap_or_else(|| request.now + Duration::minutes(i64::from(request.window_minutes)));

    if messages_currently_charged.saturating_add(request.attempted_message_cost)
        > request.configured_limit
    {
        tx.commit()?;
        return Ok(ChatGptMessageBudgetReservation::Exhausted(
            ChatGptMessageRateLimitDiagnostic {
                bucket: request.bucket,
                configured_limit: request.configured_limit,
                window_minutes: request.window_minutes,
                messages_currently_charged,
                attempted_message_cost: request.attempted_message_cost,
                remaining_seconds_until_release: seconds_until(request.now, next_release_at),
                reset_at: next_release_at,
            },
        ));
    }

    tx.execute(
        "INSERT INTO chatgpt_message_attempts
             (scope_key, message_count, charged_at)
         VALUES (?1, ?2, ?3)",
        params![
            request.scope_key,
            i64::from(request.attempted_message_cost),
            request.now.to_rfc3339()
        ],
    )
    .context("record ChatGPT message attempt")?;
    tx.commit()?;

    Ok(ChatGptMessageBudgetReservation::Charged)
}

pub(super) fn reset(conn: &mut Connection, scope_key: &str) -> Result<u64> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let deleted = tx
        .execute(
            "DELETE FROM chatgpt_message_attempts WHERE scope_key = ?1",
            [scope_key],
        )
        .context("reset ChatGPT message attempts")?;
    tx.commit()?;
    Ok(deleted as u64)
}

#[cfg(test)]
pub(super) fn current_charged_messages(
    conn: &Connection,
    scope_key: &str,
    window_minutes: u32,
    now: DateTime<Utc>,
) -> Result<u32> {
    let window_start = now - Duration::minutes(i64::from(window_minutes));
    let count = conn
        .query_row(
            "SELECT COALESCE(SUM(message_count), 0)
             FROM chatgpt_message_attempts
             WHERE scope_key = ?1 AND charged_at > ?2",
            params![scope_key, window_start.to_rfc3339()],
            |row| row.get::<_, i64>(0),
        )
        .context("count ChatGPT message attempts")?;
    Ok(count.max(0) as u32)
}

fn current_charge_count(tx: &rusqlite::Transaction<'_>, scope_key: &str) -> Result<u32> {
    let count = tx
        .query_row(
            "SELECT COALESCE(SUM(message_count), 0)
             FROM chatgpt_message_attempts
             WHERE scope_key = ?1",
            [scope_key],
            |row| row.get::<_, i64>(0),
        )
        .context("count ChatGPT message attempts")?;
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
             FROM chatgpt_message_attempts
             WHERE scope_key = ?1
             ORDER BY charged_at ASC
             LIMIT 1",
            [scope_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .context("read next ChatGPT message release")?;
    charged_at
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|parsed| {
                    parsed.with_timezone(&Utc) + Duration::minutes(i64::from(window_minutes))
                })
                .with_context(|| format!("parse ChatGPT message charged_at {value}"))
        })
        .transpose()
}

fn seconds_until(now: DateTime<Utc>, reset_at: DateTime<Utc>) -> u64 {
    (reset_at - now).num_seconds().max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use shared_protocol::RateLimitBucket;

    fn open_test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE chatgpt_message_attempts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope_key TEXT NOT NULL,
                message_count INTEGER NOT NULL,
                charged_at TEXT NOT NULL
            );
            CREATE INDEX chatgpt_message_attempts_scope_charged_at_idx
                ON chatgpt_message_attempts(scope_key, charged_at);
            "#,
        )
        .unwrap();
        conn
    }

    fn reserve_one(
        conn: &mut Connection,
        scope_key: &'static str,
        bucket: RateLimitBucket,
        configured_limit: u32,
        window_minutes: u32,
        now: DateTime<Utc>,
    ) -> ChatGptMessageBudgetReservation {
        reserve(
            conn,
            ChatGptMessageBudgetRequest {
                scope_key,
                bucket,
                configured_limit,
                window_minutes,
                attempted_message_cost: 1,
                now,
            },
        )
        .unwrap()
    }

    #[test]
    fn expired_chatgpt_message_attempts_are_pruned_before_reserving() {
        let mut conn = open_test_conn();
        let first = DateTime::parse_from_rfc3339("2026-06-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let later = first + Duration::minutes(181);

        assert!(matches!(
            reserve_one(&mut conn, "scope", RateLimitBucket::Instant, 1, 180, first),
            ChatGptMessageBudgetReservation::Charged
        ));

        assert!(matches!(
            reserve_one(&mut conn, "scope", RateLimitBucket::Instant, 1, 180, later),
            ChatGptMessageBudgetReservation::Charged
        ));

        assert_eq!(
            current_charged_messages(&conn, "scope", 180, later).unwrap(),
            1
        );
    }

    #[test]
    fn exhausted_chatgpt_message_budget_reports_oldest_attempt_release() {
        let mut conn = open_test_conn();
        let first = DateTime::parse_from_rfc3339("2026-06-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let second = first + Duration::minutes(1);

        assert!(matches!(
            reserve_one(&mut conn, "scope", RateLimitBucket::Thinking, 1, 180, first),
            ChatGptMessageBudgetReservation::Charged
        ));

        let diagnostic = match reserve_one(
            &mut conn,
            "scope",
            RateLimitBucket::Thinking,
            1,
            180,
            second,
        ) {
            ChatGptMessageBudgetReservation::Charged => panic!("reservation should be exhausted"),
            ChatGptMessageBudgetReservation::Exhausted(diagnostic) => diagnostic,
        };

        assert_eq!(diagnostic.bucket, RateLimitBucket::Thinking);
        assert_eq!(diagnostic.configured_limit, 1);
        assert_eq!(diagnostic.window_minutes, 180);
        assert_eq!(diagnostic.messages_currently_charged, 1);
        assert_eq!(diagnostic.attempted_message_cost, 1);
        assert_eq!(diagnostic.reset_at, first + Duration::minutes(180));
        assert_eq!(diagnostic.remaining_seconds_until_release, 179 * 60);
    }

    #[test]
    fn reset_chatgpt_message_budget_deletes_only_matching_scope() {
        let mut conn = open_test_conn();
        let now = DateTime::parse_from_rfc3339("2026-06-07T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert!(matches!(
            reserve_one(
                &mut conn,
                "instant-scope",
                RateLimitBucket::Instant,
                10,
                180,
                now
            ),
            ChatGptMessageBudgetReservation::Charged
        ));
        assert!(matches!(
            reserve_one(
                &mut conn,
                "thinking-scope",
                RateLimitBucket::Thinking,
                10,
                10_080,
                now
            ),
            ChatGptMessageBudgetReservation::Charged
        ));

        assert_eq!(reset(&mut conn, "instant-scope").unwrap(), 1);
        assert_eq!(
            current_charged_messages(&conn, "instant-scope", 180, now).unwrap(),
            0
        );
        assert_eq!(
            current_charged_messages(&conn, "thinking-scope", 10_080, now).unwrap(),
            1
        );
    }
}
