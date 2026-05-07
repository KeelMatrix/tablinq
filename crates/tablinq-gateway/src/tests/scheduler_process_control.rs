use super::scheduler_dsl::{
    SchedulerGateway, chat_completion_body, expect_retry_error, streaming_chat_completion_body,
};
use super::*;

#[test]
fn paperclip_like_queue_pressure_returns_retry_metadata() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
            gateway.queue_timeout_ms = Some(250);
        }
    });

    let first_body = paperclip_request_body("tablinq-research", "hold the session", false, &[]);
    let first = gateway.spawn_chat_body(first_body, None, Some("paperclip"), 20_000);

    thread::sleep(Duration::from_millis(150));

    let second_body =
        paperclip_request_body("tablinq-instant", "time out while queued", false, &[]);
    let second = gateway.post_chat_body(&second_body, None, Some("paperclip"), 20_000);
    let body = expect_retry_error(&second, 429, "queue_timeout");
    assert_eq!(body["retry_after_ms"], 250);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 200);

    gateway.shutdown();
}

#[test]
fn queued_chat_completions_wait_for_submit_jitter_before_each_provider_submit() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(fake_url.to_owned());
        config.file.browser_submit_jitter_min_ms = Some(1_000);
        config.file.browser_submit_jitter_max_ms = Some(1_000);
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.max_concurrent_upstream_jobs = Some(1);
            gateway.queue_length = Some(4);
            gateway.queue_timeout_ms = Some(15_000);
            gateway.ask_timeout_ms = Some(15_000);
        }
    });

    let first_body = chat_completion_body("tablinq-instant", "first queued ask");
    let second_body = chat_completion_body("tablinq-instant", "second queued ask");
    let first = gateway.spawn_chat_body(first_body, Some("req-submit-gap-1"), None, 20_000);

    thread::sleep(Duration::from_millis(50));

    let second_started = Instant::now();
    let second = gateway.post_chat_body(&second_body, Some("req-submit-gap-2"), None, 20_000);
    let second_elapsed = second_started.elapsed();

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 200);
    assert_eq!(second.status_code, 200);
    assert!(second_elapsed >= Duration::from_millis(1_000));

    gateway.shutdown();
}

#[test]
fn chat_completions_run_queued_requests_in_fifo_order() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(15_000);
            gateway.queue_timeout_ms = Some(60_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "hold the slot long enough to queue"),
        Some("req-first"),
        None,
        60_000,
    );

    thread::sleep(Duration::from_millis(200));

    let completions = Arc::new(Mutex::new(Vec::new()));
    let second_body = chat_completion_body("tablinq-research", "hello from second");
    let second_snapshot = gateway.snapshot.clone();
    let second_auth = gateway.auth.clone();
    let second_completions = Arc::clone(&completions);
    let second = thread::spawn(move || {
        let response = super::scheduler_dsl::post_chat_completion(
            &second_snapshot,
            &second_auth,
            &second_body,
            Some("req-second"),
            None,
            60_000,
        );
        second_completions.lock().unwrap().push("second".to_owned());
        response
    });

    thread::sleep(Duration::from_millis(100));

    let third_body = chat_completion_body("tablinq-instant", "hello from third");
    let third_snapshot = gateway.snapshot.clone();
    let third_auth = gateway.auth.clone();
    let third_completions = Arc::clone(&completions);
    let third = thread::spawn(move || {
        let response = super::scheduler_dsl::post_chat_completion(
            &third_snapshot,
            &third_auth,
            &third_body,
            Some("req-third"),
            None,
            60_000,
        );
        third_completions.lock().unwrap().push("third".to_owned());
        response
    });

    let cancel = gateway.cancel("req-first", 5_000);
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");
    let second = second.join().unwrap();
    let third = third.join().unwrap();
    assert_eq!(second.status_code, 200);
    assert_eq!(third.status_code, 200);
    assert_eq!(completions.lock().unwrap().as_slice(), ["second", "third"]);

    gateway.shutdown();
}

#[test]
fn chat_completions_cancel_queued_request() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "occupy the only slot"),
        Some("req-running"),
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(250));

    let queued = gateway.spawn_chat_body(
        chat_completion_body("tablinq-instant", "cancel me while queued"),
        Some("req-cancel-me"),
        None,
        20_000,
    );

    gateway.wait_for_scheduler(
        |scheduler| scheduler["queued"].as_u64().unwrap_or(0) >= 1,
        "queued request never reached the scheduler",
    );

    let cancel = gateway.cancel("req-cancel-me", 5_000);
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert!(cancel_body["accepted"].as_bool().is_some());
    assert!(matches!(
        cancel_body["state"].as_str(),
        Some("queued" | "running")
    ));

    let queued = queued.join().unwrap();
    assert_eq!(queued.status_code, 409);
    let queued_body: Value = serde_json::from_slice(&queued.body).unwrap();
    assert_eq!(queued_body["error"]["code"], "cancelled");

    let cancel_running = gateway.cancel("req-running", 5_000);
    assert_eq!(cancel_running.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");

    gateway.shutdown();
}

#[test]
fn chat_completions_cancel_running_research_request() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(10_000);
        }
    });

    let running = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "cancel a running research job"),
        Some("req-running-cancel"),
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(500));

    let cancel = gateway.cancel("req-running-cancel", 5_000);
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert_eq!(cancel_body["accepted"], true);
    assert_eq!(cancel_body["state"], "running");

    let running = running.join().unwrap();
    assert_eq!(running.status_code, 409);
    let body: Value = serde_json::from_slice(&running.body).unwrap();
    assert_eq!(body["error"]["code"], "cancelled");

    let summary = gateway.request_summary("req-running-cancel", 10_000);
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["status"], "cancelled");
    assert_eq!(summary_body["data"]["cancelled"], true);
    assert!(summary_body["data"]["duration_ms"].as_u64().is_some());

    gateway.shutdown();
}

#[test]
fn chat_completions_cancel_running_streaming_ask_request() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(fake_url.to_owned());
    });

    let body = streaming_chat_completion_body("tablinq-instant", "cancel a running ask stream");
    let running = gateway.spawn_chat_body(body, Some("req-stream-cancel"), None, 20_000);

    thread::sleep(Duration::from_millis(250));

    let cancel = gateway.cancel("req-stream-cancel", 5_000);
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert_eq!(cancel_body["accepted"], true);
    assert_eq!(cancel_body["state"], "running");
    assert!(
        cancel_body["message"]
            .as_str()
            .unwrap()
            .contains("cancellation requested")
    );

    let running = running.join().unwrap();
    assert_eq!(running.status_code, 200);
    let body = String::from_utf8(running.body).unwrap();
    assert!(body.contains("\"code\":\"cancelled\""));
    assert!(body.contains("[DONE]"));

    gateway.shutdown();
}

#[test]
fn chat_completions_cancel_during_reload_releases_the_queue_slot() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=reload"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.ask_timeout_ms = Some(10_000);
            gateway.queue_timeout_ms = Some(10_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-instant", "cancel me while the page reloads"),
        Some("req-reload-cancel"),
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(250));

    let cancel = gateway.cancel("req-reload-cancel", 5_000);
    assert_eq!(cancel.status_code, 200);

    let second_body =
        chat_completion_body("tablinq-instant", "run after the cancelled reload request");
    let second =
        gateway.post_chat_body(&second_body, Some("req-after-reload-cancel"), None, 20_000);
    assert_eq!(second.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");

    gateway.shutdown();
}

#[test]
fn chat_completions_return_queue_timeout_when_wait_expires() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
            gateway.queue_timeout_ms = Some(250);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "occupy the only slot"),
        None,
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(150));

    let second_body = chat_completion_body("tablinq-instant", "time out in queue");
    let second = gateway.post_chat_body(&second_body, None, None, 20_000);
    expect_retry_error(&second, 429, "queue_timeout");

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 200);

    gateway.shutdown();
}

#[test]
fn queue_reports_a_single_running_upstream_job_and_serializes_follow_up_work() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.max_concurrent_upstream_jobs = Some(1);
            gateway.queue_length = Some(4);
            gateway.queue_timeout_ms = Some(10_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "occupy the only upstream slot"),
        Some("req-single-slot-1"),
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(150));

    let second = gateway.spawn_chat_body(
        chat_completion_body("tablinq-instant", "wait behind the single slot"),
        Some("req-single-slot-2"),
        None,
        20_000,
    );

    let status_body = gateway.wait_for_scheduler(
        |scheduler| scheduler["running"] == 1 && scheduler["queued"] == 1,
        "expected running=1 and queued=1",
    );
    assert_eq!(status_body["scheduler"]["max_concurrent_upstream_jobs"], 1);

    let cancel = gateway.cancel("req-single-slot-1", 5_000);
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let second = second.join().unwrap();
    assert_eq!(second.status_code, 200);

    gateway.shutdown();
}

#[test]
fn queue_rejects_requests_when_per_client_limit_is_reached() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.per_client_limit = Some(1);
            gateway.queue_length = Some(4);
            gateway.queue_timeout_ms = Some(10_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "occupy the per-client slot"),
        Some("req-client-limit-1"),
        Some("shared-client"),
        20_000,
    );

    thread::sleep(Duration::from_millis(150));

    let second_body =
        chat_completion_body("tablinq-instant", "reject this second same-client request");
    let second = gateway.post_chat_body(&second_body, None, Some("shared-client"), 20_000);
    let second_body = expect_retry_error(&second, 429, "client_limit_exceeded");
    assert_eq!(second_body["error"]["code"], "client_limit_exceeded");
    assert!(second_body["retry_after_ms"].as_u64().is_some());

    let cancel = gateway.cancel("req-client-limit-1", 5_000);
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);

    gateway.shutdown();
}

#[test]
fn queue_rejects_requests_when_the_queue_is_full() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.queue_length = Some(1);
            gateway.queue_timeout_ms = Some(10_000);
        }
    });

    let first = gateway.spawn_chat_body(
        chat_completion_body("tablinq-research", "occupy the slot"),
        Some("req-queue-full-1"),
        None,
        20_000,
    );

    thread::sleep(Duration::from_millis(150));

    let second = gateway.spawn_chat_body(
        chat_completion_body("tablinq-instant", "sit in the only queue slot"),
        Some("req-queue-full-2"),
        None,
        20_000,
    );

    gateway.wait_for_scheduler(
        |scheduler| scheduler["queued"] == 1,
        "expected a queued request",
    );

    let third_body = chat_completion_body("tablinq-instant", "overflow the queue");
    let third = gateway.post_chat_body(&third_body, None, Some("other-client"), 20_000);
    let third_body = expect_retry_error(&third, 429, "queue_full");
    assert_eq!(third_body["error"]["code"], "queue_full");
    assert!(third_body["retry_after_ms"].as_u64().is_some());

    let cancel = gateway.cancel("req-queue-full-1", 5_000);
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let second = second.join().unwrap();
    assert_eq!(second.status_code, 200);

    gateway.shutdown();
}

#[test]
fn subagent_requests_queue_behind_primary_work_and_preserve_request_metadata() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(15_000);
            gateway.queue_timeout_ms = Some(15_000);
        }
    });

    let primary = gateway.spawn_chat_body(
        openclaude_request_body(
            "tablinq-research",
            "hold the browser session",
            "primary",
            false,
            None,
        ),
        Some("req-subagent-primary"),
        Some("openclaude"),
        20_000,
    );

    thread::sleep(Duration::from_millis(250));

    let queued = gateway.spawn_chat_body(
        openclaude_request_body(
            "tablinq-instant",
            "wait as a queued subagent request",
            "subagent",
            false,
            Some("req-subagent-primary"),
        ),
        Some("req-subagent-queued"),
        Some("openclaude"),
        20_000,
    );

    gateway.wait_for_scheduler(
        |scheduler| scheduler["running"] == 1 && scheduler["queued"] == 1,
        "expected the subagent request to queue behind the primary request",
    );

    let cancel = gateway.cancel("req-subagent-primary", 5_000);
    assert_eq!(cancel.status_code, 200);

    let primary = primary.join().unwrap();
    assert_eq!(primary.status_code, 409);
    let queued = queued.join().unwrap();
    assert_eq!(queued.status_code, 200);
    let queued_body: Value = serde_json::from_slice(&queued.body).unwrap();
    assert_eq!(queued_body["tablinq_request_role"], "subagent");
    assert_eq!(
        queued_body["tablinq_parent_request_id"],
        "req-subagent-primary"
    );

    let summary = gateway.request_summary("req-subagent-queued", 10_000);
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["request_role"], "subagent");
    assert_eq!(
        summary_body["data"]["parent_request_id"],
        "req-subagent-primary"
    );

    gateway.shutdown();
}

#[test]
fn queued_subagent_requests_surface_queue_timeout_metadata_instead_of_policy_refusal() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let gateway = SchedulerGateway::start(|config, fake_url| {
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
            gateway.queue_timeout_ms = Some(250);
        }
    });

    let primary = gateway.spawn_chat_body(
        openclaude_request_body(
            "tablinq-research",
            "hold the browser session",
            "primary",
            false,
            None,
        ),
        Some("req-subagent-timeout-primary"),
        Some("openclaude"),
        20_000,
    );

    thread::sleep(Duration::from_millis(150));

    let queued = gateway.post_chat_body(
        &openclaude_request_body(
            "tablinq-instant",
            "time out as a queued subagent request",
            "subagent",
            false,
            Some("req-subagent-timeout-primary"),
        ),
        Some("req-subagent-timeout"),
        Some("openclaude"),
        20_000,
    );
    let queued_body = expect_retry_error(&queued, 429, "queue_timeout");
    assert_eq!(queued_body["request_id"], "req-subagent-timeout");
    assert_eq!(queued_body["job_id"].as_str().is_some(), true);
    assert_eq!(queued_body["retry_after_ms"], 250);
    assert_ne!(queued_body["error"]["code"], "subagents_disabled");
    assert!(
        queued_body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("queue timeout")
    );

    let summary = gateway.request_summary("req-subagent-timeout", 10_000);
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["request_role"], "subagent");
    assert_eq!(
        summary_body["data"]["parent_request_id"],
        "req-subagent-timeout-primary"
    );

    let cancel = gateway.cancel("req-subagent-timeout-primary", 5_000);
    assert_eq!(cancel.status_code, 200);
    let primary = primary.join().unwrap();
    assert_eq!(primary.status_code, 409);

    gateway.shutdown();
}
