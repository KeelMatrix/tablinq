use crate::http::{HttpParseError, HttpRequest, HttpResponse, finalized_http_response_headers};
use crate::routes::{Route, match_route};
use crate::sse::sse_response_headers;
use chrono::{DateTime, Utc};
use gptd::db::{
    GATEWAY_HTTP_STREAMING_RESPONSE_REASON, GatewayHttpCapturedBody, GatewayHttpRequestFinish,
    GatewayHttpRequestStart, capture_gateway_http_body, gateway_http_headers_json,
};
use gptd::logging::JsonLogger;
use gptd::{AppConfig, Database};
use serde_json::json;
use std::path::PathBuf;

pub(crate) enum GatewayHttpResponseCapture {
    Body {
        status_code: u16,
        headers_json: Option<String>,
        body: GatewayHttpCapturedBody,
    },
    Omitted {
        status_code: Option<u16>,
        headers_json: Option<String>,
        reason: &'static str,
    },
}

pub(crate) struct GatewayHttpMonitor {
    db_path: PathBuf,
    row_id: Option<i64>,
    request_id: String,
    started_at: DateTime<Utc>,
}

impl GatewayHttpResponseCapture {
    pub(crate) fn from_http_response(request_id: &str, response: &HttpResponse) -> Self {
        let headers = finalized_http_response_headers(request_id, response);
        Self::Body {
            status_code: response.status_code,
            headers_json: headers_json(
                headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
            body: capture_gateway_http_body(&response.body),
        }
    }

    pub(crate) fn streaming(request_id: &str) -> Self {
        let headers = sse_response_headers(request_id);
        Self::Omitted {
            status_code: Some(200),
            headers_json: headers_json(
                headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
            reason: GATEWAY_HTTP_STREAMING_RESPONSE_REASON,
        }
    }

    pub(crate) fn no_response(reason: &'static str) -> Self {
        Self::Omitted {
            status_code: None,
            headers_json: None,
            reason,
        }
    }
}

impl GatewayHttpMonitor {
    pub(crate) fn start(
        config: &AppConfig,
        logger: &JsonLogger,
        request_id: &str,
        remote_addr: &str,
        request: &HttpRequest,
        route: Route,
    ) -> Self {
        let started_at = Utc::now();
        let start = GatewayHttpRequestStart {
            request_id: Some(request_id.to_owned()),
            remote_addr: Some(remote_addr.to_owned()),
            method: request.method.clone(),
            path: request.path.clone(),
            route: Some(format!("{route:?}")),
            request_headers_json: headers_json(
                request
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
            request_body: Some(capture_gateway_http_body(&request.body)),
            parse_error: None,
            started_at,
            created_at: started_at,
        };
        Self::insert(config, logger, request_id, started_at, start)
    }

    pub(crate) fn start_parse_failure(
        config: &AppConfig,
        logger: &JsonLogger,
        request_id: &str,
        remote_addr: &str,
        err: &HttpParseError,
    ) -> Self {
        let started_at = Utc::now();
        let method = err.partial.method.clone().unwrap_or_default();
        let path = err.partial.path.clone().unwrap_or_default();
        let route = if method.is_empty() || path.is_empty() {
            None
        } else {
            Some(format!("{:?}", match_route(&method, &path)))
        };
        let start = GatewayHttpRequestStart {
            request_id: Some(request_id.to_owned()),
            remote_addr: Some(remote_addr.to_owned()),
            method,
            path,
            route,
            request_headers_json: headers_json(
                err.partial
                    .headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_str())),
            ),
            request_body: err
                .partial
                .body
                .as_ref()
                .map(|body| capture_gateway_http_body(body)),
            parse_error: Some(format!("{err:#}")),
            started_at,
            created_at: started_at,
        };
        Self::insert(config, logger, request_id, started_at, start)
    }

    pub(crate) fn finish(&mut self, logger: &JsonLogger, response: GatewayHttpResponseCapture) {
        let Some(row_id) = self.row_id.take() else {
            return;
        };
        let finished_at = Utc::now();
        let duration_ms = (finished_at - self.started_at).num_milliseconds().max(0) as u64;
        let finish = match response {
            GatewayHttpResponseCapture::Body {
                status_code,
                headers_json,
                body,
            } => GatewayHttpRequestFinish {
                response_status_code: Some(status_code),
                response_headers_json: headers_json,
                response_body: Some(body),
                response_capture_omitted_reason: None,
                finished_at,
                duration_ms,
            },
            GatewayHttpResponseCapture::Omitted {
                status_code,
                headers_json,
                reason,
            } => GatewayHttpRequestFinish {
                response_status_code: status_code,
                response_headers_json: headers_json,
                response_body: None,
                response_capture_omitted_reason: Some(reason.to_owned()),
                finished_at,
                duration_ms,
            },
        };
        match Database::open(&self.db_path)
            .and_then(|db| db.finish_gateway_http_request(row_id, &finish))
        {
            Ok(()) => {}
            Err(err) => warn_monitoring_failure(
                logger,
                "gateway HTTP monitoring finish failed",
                &self.request_id,
                err,
            ),
        }
    }

    fn insert(
        config: &AppConfig,
        logger: &JsonLogger,
        request_id: &str,
        started_at: DateTime<Utc>,
        start: GatewayHttpRequestStart,
    ) -> Self {
        let db_path = config.paths.db_path.clone();
        let row_id =
            match Database::open(&db_path).and_then(|db| db.start_gateway_http_request(&start)) {
                Ok(row_id) => Some(row_id),
                Err(err) => {
                    warn_monitoring_failure(
                        logger,
                        "gateway HTTP monitoring start failed",
                        request_id,
                        err,
                    );
                    None
                }
            };
        Self {
            db_path,
            row_id,
            request_id: request_id.to_owned(),
            started_at,
        }
    }
}

fn headers_json<'a, I>(headers: I) -> Option<String>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    gateway_http_headers_json(headers).ok()
}

fn warn_monitoring_failure(
    logger: &JsonLogger,
    message: &'static str,
    request_id: &str,
    err: anyhow::Error,
) {
    let _ = logger.warn(
        message,
        json!({
            "request_id": request_id,
            "error": format!("{err:#}"),
        }),
    );
}
