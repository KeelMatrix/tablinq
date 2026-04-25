use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{ConversationSessionInfo, SessionList, SessionSummary};

use super::{conversations, row_mapping::parse_timestamp};

pub(crate) fn list(conn: &Connection) -> Result<SessionList> {
    let mut statement = conn.prepare(
        "SELECT session_id, label, tab_url, last_seen_at FROM sessions ORDER BY last_seen_at DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            parse_timestamp(row.get::<_, String>(3)?)?,
        ))
    })?;

    let mut sessions = Vec::new();
    for row in rows {
        let (session_id, label, tab_url, last_seen_at) = row?;
        let recent_conversations = conversations::recent_for_session(conn, &session_id, 10)?;
        let latest_conversation_id = recent_conversations
            .first()
            .map(|conversation| conversation.conversation_id.clone());
        sessions.push(SessionSummary {
            session_id,
            label,
            tab_url,
            last_seen_at,
            latest_conversation_id,
            recent_conversations,
        });
    }
    Ok(SessionList { sessions })
}

pub(crate) fn find_info(
    conn: &Connection,
    session_id: &str,
) -> Result<Option<ConversationSessionInfo>> {
    conn.query_row(
        r#"
        SELECT session_id, label, tab_url, last_seen_at
        FROM sessions
        WHERE session_id = ?1
        "#,
        [session_id],
        |row| {
            Ok(ConversationSessionInfo {
                session_id: row.get(0)?,
                label: row.get(1)?,
                tab_url: row.get(2)?,
                last_seen_at: parse_timestamp(row.get::<_, String>(3)?)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn upsert(
    conn: &Connection,
    session_id: &str,
    label: &str,
    tab_url: Option<&str>,
    last_seen_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO sessions (session_id, label, tab_url, last_seen_at)
        VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT(session_id) DO UPDATE SET
            label = excluded.label,
            tab_url = excluded.tab_url,
            last_seen_at = excluded.last_seen_at
        "#,
        params![session_id, label, tab_url, last_seen_at.to_rfc3339()],
    )?;
    Ok(())
}
