use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::FailureBundle;

pub(crate) fn insert(conn: &Connection, bundle: &FailureBundle) -> Result<()> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO failure_bundles (bundle_id, created_at, payload_json)
        VALUES (?1, ?2, ?3)
        "#,
        params![
            bundle.bundle_id,
            bundle.created_at.to_rfc3339(),
            serde_json::to_string(bundle)?,
        ],
    )?;
    Ok(())
}

pub(crate) fn count(conn: &Connection) -> Result<u64> {
    let count = conn.query_row("SELECT COUNT(*) FROM failure_bundles", [], |row| {
        row.get::<_, u64>(0)
    })?;
    Ok(count)
}

pub(crate) fn latest(conn: &Connection) -> Result<Option<FailureBundle>> {
    let payload = conn
        .query_row(
            "SELECT payload_json FROM failure_bundles ORDER BY created_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    payload
        .map(|raw| serde_json::from_str(&raw).context("decode failure bundle from sqlite"))
        .transpose()
}
