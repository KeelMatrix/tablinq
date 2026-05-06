use crate::http::{HttpRequest, HttpResponse};
use crate::json::json_response;
use crate::openai::openai_error;
use gptd::AppConfig;
use serde_json::json;
use std::net::SocketAddr;

pub(crate) fn authorize_request(
    config: &AppConfig,
    request: &HttpRequest,
    peer_addr: Option<SocketAddr>,
    openai_shape: bool,
) -> std::result::Result<(), HttpResponse> {
    if !peer_addr.is_some_and(|addr| addr.ip().is_loopback()) {
        return Err(json_response(
            403,
            json!({
                "error": "forbidden",
            }),
        ));
    }

    let Some(expected_api_key) = config.gateway_api_key() else {
        return Err(unauthorized_response(openai_shape));
    };
    let provided = request.headers.get("authorization").map(String::as_str);
    if bearer_token_matches(provided, expected_api_key) {
        Ok(())
    } else {
        Err(unauthorized_response(openai_shape))
    }
}

fn unauthorized_response(openai_shape: bool) -> HttpResponse {
    if openai_shape {
        json_response(
            401,
            openai_error(
                401,
                "invalid_request_error",
                "invalid_api_key",
                "missing or invalid gateway bearer token",
            ),
        )
    } else {
        json_response(
            401,
            json!({
                "error": "unauthorized",
            }),
        )
    }
}

pub(crate) fn bearer_token_matches(provided: Option<&str>, expected_api_key: &str) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let Some(provided_token) = provided.strip_prefix("Bearer ") else {
        return false;
    };
    constant_time_bytes_eq(provided_token.as_bytes(), expected_api_key.as_bytes())
}

pub(crate) fn constant_time_bytes_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let left_byte = *left.get(index).unwrap_or(&0);
        let right_byte = *right.get(index).unwrap_or(&0);
        diff |= usize::from(left_byte ^ right_byte);
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::net::{Ipv4Addr, SocketAddr};

    #[test]
    fn bearer_token_matching_requires_exact_bearer_token() {
        assert!(bearer_token_matches(Some("Bearer secret"), "secret"));
        assert!(!bearer_token_matches(Some("Bearer wrong"), "secret"));
        assert!(!bearer_token_matches(Some("secret"), "secret"));
        assert!(!bearer_token_matches(None, "secret"));
    }

    #[test]
    fn openai_auth_failure_keeps_openai_error_shape() {
        let dir = tempfile::tempdir().expect("create temp state root");
        let mut config = AppConfig::load_or_create(Some(dir.path())).expect("load config");
        config
            .file
            .gateway
            .as_mut()
            .expect("gateway config")
            .api_key = Some("secret".to_owned());

        let response = authorize_request(
            &config,
            &request_with_auth(Some("Bearer wrong")),
            Some(loopback_peer()),
            true,
        )
        .expect_err("invalid token");
        assert_eq!(response.status_code, 401);
        let body: serde_json::Value =
            serde_json::from_slice(&response.body).expect("decode auth response");
        assert_eq!(body["error"]["code"], "invalid_api_key");
    }

    #[test]
    fn missing_configured_api_key_fails_closed() {
        let dir = tempfile::tempdir().expect("create temp state root");
        let mut config = AppConfig::load_or_create(Some(dir.path())).expect("load config");
        config
            .file
            .gateway
            .as_mut()
            .expect("gateway config")
            .api_key = Some("   ".to_owned());

        let response = authorize_request(
            &config,
            &request_with_auth(Some("Bearer secret")),
            Some(loopback_peer()),
            false,
        )
        .expect_err("missing configured api key should fail closed");
        assert_eq!(response.status_code, 401);
    }

    #[test]
    fn missing_authorization_is_rejected() {
        let config = config_with_gateway_api_key("secret");
        let response = authorize_request(
            &config,
            &request_with_auth(None),
            Some(loopback_peer()),
            false,
        )
        .expect_err("missing authorization");
        assert_eq!(response.status_code, 401);
    }

    #[test]
    fn wrong_authorization_is_rejected() {
        let config = config_with_gateway_api_key("secret");
        let response = authorize_request(
            &config,
            &request_with_auth(Some("Bearer wrong")),
            Some(loopback_peer()),
            false,
        )
        .expect_err("wrong authorization");
        assert_eq!(response.status_code, 401);
    }

    #[test]
    fn correct_authorization_from_loopback_succeeds() {
        let config = config_with_gateway_api_key("secret");
        authorize_request(
            &config,
            &request_with_auth(Some("Bearer secret")),
            Some(loopback_peer()),
            false,
        )
        .expect("loopback request with matching bearer token");
    }

    #[test]
    fn non_loopback_peer_is_rejected() {
        let config = config_with_gateway_api_key("secret");
        let response = authorize_request(
            &config,
            &request_with_auth(Some("Bearer secret")),
            Some(SocketAddr::from((Ipv4Addr::new(192, 168, 1, 25), 9000))),
            false,
        )
        .expect_err("non-loopback peer");
        assert_eq!(response.status_code, 403);
    }

    fn config_with_gateway_api_key(api_key: &str) -> AppConfig {
        let dir = tempfile::tempdir().expect("create temp state root");
        let mut config = AppConfig::load_or_create(Some(dir.path())).expect("load config");
        config
            .file
            .gateway
            .as_mut()
            .expect("gateway config")
            .api_key = Some(api_key.to_owned());
        config
    }

    fn request_with_auth(authorization: Option<&str>) -> HttpRequest {
        let mut headers = BTreeMap::new();
        if let Some(value) = authorization {
            headers.insert("authorization".to_owned(), value.to_owned());
        }
        HttpRequest {
            method: "GET".to_owned(),
            path: "/v1/models".to_owned(),
            headers,
            body: Vec::new(),
        }
    }

    fn loopback_peer() -> SocketAddr {
        SocketAddr::from((Ipv4Addr::LOCALHOST, 9000))
    }
}
