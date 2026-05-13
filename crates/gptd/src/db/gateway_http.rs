use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const GATEWAY_HTTP_BODY_CAPTURE_LIMIT: usize = 256 * 1024;
pub const GATEWAY_HTTP_STREAMING_RESPONSE_REASON: &str = "streaming_response";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayHttpCapturedBody {
    pub text: String,
    pub sha256: String,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct GatewayHttpRequestStart {
    pub request_id: Option<String>,
    pub remote_addr: Option<String>,
    pub method: String,
    pub path: String,
    pub route: Option<String>,
    pub request_headers_json: Option<String>,
    pub request_body: Option<GatewayHttpCapturedBody>,
    pub parse_error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct GatewayHttpRequestFinish {
    pub response_status_code: Option<u16>,
    pub response_headers_json: Option<String>,
    pub response_body: Option<GatewayHttpCapturedBody>,
    pub response_capture_omitted_reason: Option<String>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayHttpRequestRecord {
    pub id: i64,
    pub request_id: Option<String>,
    pub remote_addr: Option<String>,
    pub method: String,
    pub path: String,
    pub route: Option<String>,
    pub request_headers_json: Option<String>,
    pub request_body_text: Option<String>,
    pub request_body_sha256: Option<String>,
    pub request_body_truncated: bool,
    pub response_status_code: Option<u16>,
    pub response_headers_json: Option<String>,
    pub response_body_text: Option<String>,
    pub response_body_sha256: Option<String>,
    pub response_body_truncated: bool,
    pub response_capture_omitted_reason: Option<String>,
    pub parse_error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize)]
struct HeaderRecord<'a> {
    name: &'a str,
    value: String,
}

pub fn capture_gateway_http_body(bytes: &[u8]) -> GatewayHttpCapturedBody {
    let truncated = bytes.len() > GATEWAY_HTTP_BODY_CAPTURE_LIMIT;
    let captured_len = bytes.len().min(GATEWAY_HTTP_BODY_CAPTURE_LIMIT);
    GatewayHttpCapturedBody {
        text: String::from_utf8_lossy(&bytes[..captured_len]).into_owned(),
        sha256: sha256_hex(bytes),
        truncated,
    }
}

pub fn gateway_http_headers_json<'a, I>(headers: I) -> Result<String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let headers = headers
        .into_iter()
        .map(|(name, value)| HeaderRecord {
            name,
            value: if is_sensitive_header(name) {
                "[redacted]".to_owned()
            } else {
                value.to_owned()
            },
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&headers).context("serialize gateway HTTP headers")
}

pub(crate) fn insert_start(conn: &Connection, start: &GatewayHttpRequestStart) -> Result<i64> {
    let request_body_text = start.request_body.as_ref().map(|body| body.text.as_str());
    let request_body_sha256 = start.request_body.as_ref().map(|body| body.sha256.as_str());
    let request_body_truncated = start
        .request_body
        .as_ref()
        .map(|body| if body.truncated { 1 } else { 0 })
        .unwrap_or(0);
    conn.execute(
        r#"
        INSERT INTO gateway_http_requests (
            request_id,
            remote_addr,
            method,
            path,
            route,
            request_headers_json,
            request_body_text,
            request_body_sha256,
            request_body_truncated,
            parse_error,
            started_at,
            created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        "#,
        params![
            start.request_id.as_deref(),
            start.remote_addr.as_deref(),
            start.method.as_str(),
            start.path.as_str(),
            start.route.as_deref(),
            start.request_headers_json.as_deref(),
            request_body_text,
            request_body_sha256,
            request_body_truncated,
            start.parse_error.as_deref(),
            start.started_at.to_rfc3339(),
            start.created_at.to_rfc3339(),
        ],
    )
    .context("insert gateway HTTP request monitor row")?;
    Ok(conn.last_insert_rowid())
}

pub(crate) fn finish(conn: &Connection, id: i64, finish: &GatewayHttpRequestFinish) -> Result<()> {
    let response_body_text = finish.response_body.as_ref().map(|body| body.text.as_str());
    let response_body_sha256 = finish
        .response_body
        .as_ref()
        .map(|body| body.sha256.as_str());
    let response_body_truncated = finish
        .response_body
        .as_ref()
        .map(|body| if body.truncated { 1 } else { 0 })
        .unwrap_or(0);
    conn.execute(
        r#"
        UPDATE gateway_http_requests
        SET response_status_code = ?2,
            response_headers_json = ?3,
            response_body_text = ?4,
            response_body_sha256 = ?5,
            response_body_truncated = ?6,
            response_capture_omitted_reason = ?7,
            finished_at = ?8,
            duration_ms = ?9
        WHERE id = ?1
        "#,
        params![
            id,
            finish.response_status_code.map(i64::from),
            finish.response_headers_json.as_deref(),
            response_body_text,
            response_body_sha256,
            response_body_truncated,
            finish.response_capture_omitted_reason.as_deref(),
            finish.finished_at.to_rfc3339(),
            i64::try_from(finish.duration_ms).unwrap_or(i64::MAX),
        ],
    )
    .with_context(|| format!("finish gateway HTTP request monitor row {id}"))?;
    Ok(())
}

pub(crate) fn latest_by_request_id(
    conn: &Connection,
    request_id: &str,
) -> Result<Option<GatewayHttpRequestRecord>> {
    conn.query_row(
        r#"
        SELECT id,
               request_id,
               remote_addr,
               method,
               path,
               route,
               request_headers_json,
               request_body_text,
               request_body_sha256,
               request_body_truncated,
               response_status_code,
               response_headers_json,
               response_body_text,
               response_body_sha256,
               response_body_truncated,
               response_capture_omitted_reason,
               parse_error,
               started_at,
               finished_at,
               duration_ms,
               created_at
        FROM gateway_http_requests
        WHERE request_id = ?1
        ORDER BY id DESC
        LIMIT 1
        "#,
        [request_id],
        map_record,
    )
    .optional()
    .context("query gateway HTTP request monitor row")
}

pub(crate) fn count(conn: &Connection) -> Result<u64> {
    let count = conn
        .query_row("SELECT COUNT(*) FROM gateway_http_requests", [], |row| {
            row.get::<_, i64>(0)
        })
        .context("count gateway HTTP request monitor rows")?;
    Ok(count.max(0) as u64)
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<GatewayHttpRequestRecord> {
    let started_at = parse_timestamp(row.get(17)?, 17)?;
    let finished_at = row
        .get::<_, Option<String>>(18)?
        .map(|value| parse_timestamp(value, 18))
        .transpose()?;
    let duration_ms = row
        .get::<_, Option<i64>>(19)?
        .map(|value| value.max(0) as u64);
    let created_at = parse_timestamp(row.get(20)?, 20)?;
    Ok(GatewayHttpRequestRecord {
        id: row.get(0)?,
        request_id: row.get(1)?,
        remote_addr: row.get(2)?,
        method: row.get(3)?,
        path: row.get(4)?,
        route: row.get(5)?,
        request_headers_json: row.get(6)?,
        request_body_text: row.get(7)?,
        request_body_sha256: row.get(8)?,
        request_body_truncated: row.get::<_, i64>(9)? != 0,
        response_status_code: row.get::<_, Option<i64>>(10)?.map(|value| value as u16),
        response_headers_json: row.get(11)?,
        response_body_text: row.get(12)?,
        response_body_sha256: row.get(13)?,
        response_body_truncated: row.get::<_, i64>(14)? != 0,
        response_capture_omitted_reason: row.get(15)?,
        parse_error: row.get(16)?,
        started_at,
        finished_at,
        duration_ms,
        created_at,
    })
}

fn is_sensitive_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-api-key"
    )
}

fn parse_timestamp(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    value.parse::<DateTime<Utc>>().map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
