use super::*;
use chrono::{TimeZone, Utc};
use shared_protocol::{JobKind, JobStatus};

const V1_STATUS_PATH: &str = "/v1/status";
const V1_DOCTOR_PATH: &str = "/v1/doctor";
const V1_CONFIG_PATH: &str = "/v1/config";
const V1_JOBS_PATH: &str = "/v1/jobs";

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 1, 12, 0, 0).unwrap()
}

fn seed_jobs_fixture(config: &AppConfig) {
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = fixed_time();
    db.insert_job(
        "job-running",
        JobStatus::Running,
        Some("processing"),
        Some("conv-1"),
        now,
        now + chrono::Duration::minutes(1),
    )
    .unwrap();
    db.upsert_job_detail("job-running", JobKind::Ask, Some("prompt one"), None)
        .unwrap();
}

fn seed_completed_jobs_fixture(config: &AppConfig) {
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = fixed_time();
    db.insert_job(
        "job-complete",
        JobStatus::Completed,
        Some("done"),
        Some("conv-2"),
        now + chrono::Duration::minutes(2),
        now + chrono::Duration::minutes(3),
    )
    .unwrap();
    db.upsert_job_detail("job-complete", JobKind::Research, Some("prompt two"), None)
        .unwrap();
    db.update_job_output(
        "job-complete",
        Some("report ready"),
        Some("https://chatgpt.com/c/conv-2"),
        None,
    )
    .unwrap();
}

#[test]
fn v1_status_succeeds_with_auth_and_hides_api_key() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let raw_api_key = config.gateway_api_key().unwrap().to_owned();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_STATUS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.status");
    assert_eq!(body["data"]["gateway"]["api_key_present"], true);
    assert!(body["data"]["daemon"]["state_root"].is_string());
    assert!(!String::from_utf8_lossy(&response.body).contains(&raw_api_key));
}

#[test]
fn v1_doctor_succeeds_with_auth() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_DOCTOR_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.doctor");
    assert!(body["data"]["status"]["state_root"].is_string());
    assert_eq!(body["scheduler"]["max_concurrent_upstream_jobs"], 1);
}

#[test]
fn v1_config_succeeds_with_auth_and_includes_local_paths() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let raw_api_key = config.gateway_api_key().unwrap().to_owned();
    let expected_state_root = config.paths.state_root.display().to_string();
    let expected_config_path = config.paths.config_path.display().to_string();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_CONFIG_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();

    let body = assert_json_status(&response, 200);
    assert_eq!(body["object"], "tablinq.config");
    assert_eq!(body["data"]["state_root"], expected_state_root);
    assert_eq!(body["data"]["config_path"], expected_config_path);
    assert_eq!(body["data"]["gateway"]["api_key_present"], true);
    assert!(!String::from_utf8_lossy(&response.body).contains(&raw_api_key));
}

#[test]
fn v1_jobs_endpoints_list_detail_cancel_and_not_found() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_completed_jobs_fixture(&config);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config.clone());
    let snapshot = &gateway.snapshot;
    seed_jobs_fixture(&config);

    let list = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_JOBS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let list_body = assert_json_status(&list, 200);
    assert_eq!(list_body["object"], "tablinq.job_list");
    assert!(
        list_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["job_id"] == "job-running")
    );

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/jobs/job-complete",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let detail_body = assert_json_status(&detail, 200);
    assert_eq!(detail_body["object"], "tablinq.job");
    assert_eq!(detail_body["data"]["job_id"], "job-complete");
    assert_eq!(detail_body["data"]["status"], "completed");

    let missing = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/jobs/job-missing",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let missing_body = assert_json_status(&missing, 404);
    assert_eq!(missing_body["error"], "not_found");

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/jobs/job-running/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let cancel_body = assert_json_status(&cancel, 200);
    assert_eq!(cancel_body["object"], "tablinq.ack");
    assert_eq!(cancel_body["job_id"], "job-running");
    assert_eq!(cancel_body["accepted"], true);

    let db = Database::open(&config.paths.db_path).unwrap();
    let job = db.job_detail("job-running").unwrap().unwrap();
    assert!(job.cancel_requested);
}

#[test]
fn v1_safe_core_endpoints_require_authorization() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_jobs_fixture(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    for (method, path) in [
        ("GET", V1_STATUS_PATH),
        ("GET", V1_DOCTOR_PATH),
        ("GET", V1_CONFIG_PATH),
        ("GET", V1_JOBS_PATH),
        ("GET", "/v1/jobs/job-running"),
        ("POST", "/v1/jobs/job-running/cancel"),
    ] {
        let response =
            http_request(&snapshot.host, snapshot.port, method, path, &[], &[], 5_000).unwrap();
        assert_eq!(response.status_code, 401, "{method} {path}");
    }
}

#[test]
fn v1_safe_core_endpoints_return_consistent_method_errors() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_jobs_fixture(&config);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let status_wrong_method = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_STATUS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let status_body = assert_json_status(&status_wrong_method, 405);
    assert_eq!(status_body["error"], "method_not_allowed");

    let cancel_wrong_method = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/jobs/job-running/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let cancel_body = assert_json_status(&cancel_wrong_method, 405);
    assert_eq!(cancel_body["error"], "method_not_allowed");
}
