use super::*;

const V1_BROWSER_ATTACH_PATH: &str = "/v1/browser/attach";
const V1_BROWSER_DETACH_PATH: &str = "/v1/browser/detach";
const V1_BROWSER_PROFILE_INFO_PATH: &str = "/v1/browser/profile-info";
const V1_BROWSER_CONNECTION_INFO_PATH: &str = "/v1/browser/connection-info";

#[test]
fn browser_attach_accepts_empty_body_and_optional_json_body() {
    require_browser_integration_prereqs!();
    let _guard = integration_lock();
    let (_fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url.clone());
    let normalized_target = format!("{fake_url}/");
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let empty_body = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &[],
        20_000,
    )
    .unwrap();
    let empty_json = assert_json_status(&empty_body, 200);
    assert_eq!(empty_json["object"], "tablinq.browser");
    assert_eq!(empty_json["data"]["running"], true);
    assert_eq!(empty_json["data"]["target_url"], normalized_target);

    let explicit_body = serde_json::to_vec(&json!({
        "target_url": fake_url,
        "browser_path": null,
        "headless": false,
    }))
    .unwrap();
    let explicit = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &explicit_body,
        20_000,
    )
    .unwrap();
    let explicit_json = assert_json_status(&explicit, 200);
    assert_eq!(explicit_json["data"]["running"], true);
    assert_eq!(explicit_json["data"]["target_url"], normalized_target);
}

#[test]
fn browser_attach_accepts_empty_json_object() {
    require_browser_integration_prereqs!();
    let _guard = integration_lock();
    let (_fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url.clone());
    let normalized_target = format!("{fake_url}/");
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{}"#,
        20_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.browser");
    assert_eq!(body["data"]["running"], true);
    assert_eq!(body["data"]["target_url"], normalized_target);
}

#[test]
fn browser_attach_rejects_unknown_fields() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{"unexpected":true}"#,
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 400);
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_request_json");
    assert_eq!(body["error"]["param"], Value::Null);
}

#[test]
fn browser_attach_invalid_json_returns_specific_error() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{"target_url":"https://chatgpt.com/""#,
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 400);
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_request_json");
    assert_eq!(body["error"]["param"], Value::Null);
}

#[test]
fn browser_detach_invalid_json_returns_nested_error() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_DETACH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{"unexpected":true}"#,
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 400);
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_request_json");
    assert_eq!(body["error"]["param"], Value::Null);
}

#[test]
fn browser_detach_invokes_core_browser_close() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_DETACH_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.browser");
    assert_eq!(body["data"]["running"], false);
    assert!(body["data"]["action"].is_string());
}

#[test]
fn browser_profile_info_returns_core_snapshot() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let expected_target = config.browser_target_url().to_owned();
    let expected_debug_endpoint = config.browser_debug_endpoint();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_BROWSER_PROFILE_INFO_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.browser");
    assert_eq!(body["data"]["running"], false);
    assert_eq!(body["data"]["target_url"], expected_target);
    assert_eq!(body["data"]["debug_endpoint"], expected_debug_endpoint);
}

#[test]
fn browser_connection_info_returns_config_derived_shape_without_running_browser() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let expected_target = config.browser_target_url().to_owned();
    let expected_debug_endpoint = config.browser_debug_endpoint();
    let expected_channel = config.browser_channel().to_owned();
    let expected_path = config.browser_path().map(str::to_owned);
    let expected_extension_required = config.browser_extension_required();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_BROWSER_CONNECTION_INFO_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.browser_connection_info");
    assert_eq!(body["debug_endpoint"], expected_debug_endpoint);
    assert_eq!(body["target_url"], expected_target);
    assert_eq!(body["browser_channel"], expected_channel);
    assert_eq!(
        body["browser_path"],
        expected_path.map_or(Value::Null, Value::String)
    );
    assert_eq!(body["extension_required"], expected_extension_required);
    assert_eq!(body["browser"]["running"], false);
    assert!(
        body["recommended_cli_setup"]
            .as_str()
            .unwrap()
            .contains("gpt browser start-debug")
    );
}

#[test]
fn browser_lifecycle_routes_reject_wrong_methods_and_missing_authorization() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let wrong_method = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_BROWSER_ATTACH_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let wrong_method_body = assert_json_status(&wrong_method, 405);
    assert!(wrong_method_body["request_id"].as_str().is_some());
    assert_eq!(wrong_method_body["error"]["code"], "method_not_allowed");

    let missing_auth = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_BROWSER_CONNECTION_INFO_PATH,
        &[],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(missing_auth.status_code, 401);

    let missing_attach_auth = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_BROWSER_ATTACH_PATH,
        &[("content-type", "application/json")],
        br#"{"target_url":"https://chatgpt.com/""#,
        5_000,
    )
    .unwrap();
    assert_eq!(missing_attach_auth.status_code, 401);
}
