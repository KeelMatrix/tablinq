#![cfg(windows)]

use gptd::AppConfig;
use serde_json::Value;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::{NamedTempFile, tempdir};

fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn powershell() -> String {
    for candidate in ["pwsh", "powershell"] {
        let status = Command::new(candidate)
            .arg("-NoLogo")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("$PSVersionTable.PSVersion.Major")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(result) if result.success()) {
            return candidate.to_owned();
        }
    }

    panic!("missing PowerShell executable");
}

fn package_root_override() -> Option<PathBuf> {
    std::env::var_os("TABLINQ_PACKAGE_ROOT").map(PathBuf::from)
}

fn run_success(mut command: Command) -> String {
    let stdout_file = NamedTempFile::new().expect("create stdout temp file");
    let stderr_file = NamedTempFile::new().expect("create stderr temp file");
    let stdout_path = stdout_file.path().to_path_buf();
    let stderr_path = stderr_file.path().to_path_buf();
    command.stdout(Stdio::from(
        stdout_file.reopen().expect("reopen stdout temp file"),
    ));
    command.stderr(Stdio::from(
        stderr_file.reopen().expect("reopen stderr temp file"),
    ));

    let mut child = command.spawn().expect("run command");
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll command") {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "command timed out: stdout={} stderr={}",
                String::from_utf8_lossy(&fs::read(&stdout_path).unwrap_or_default()),
                String::from_utf8_lossy(&fs::read(&stderr_path).unwrap_or_default())
            );
        }
        thread::sleep(Duration::from_millis(100));
    };

    let stdout = fs::read(&stdout_path).expect("read stdout temp file");
    let stderr = fs::read(&stderr_path).expect("read stderr temp file");
    if !status.success() {
        panic!(
            "command failed: status={} stdout={} stderr={}",
            status,
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
    }
    String::from_utf8(stdout).expect("utf8 stdout")
}

fn wait_for_daemon(state_root: &Path) {
    let config = AppConfig::load_or_create(Some(state_root)).expect("load daemon config");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("daemon did not become responsive");
}

fn wait_for_packaged_gateway(
    installed_gpt: &Path,
    installed_gptd: &Path,
    installed_gateway: &Path,
    state_root: &Path,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let output = run_success({
            let mut command = Command::new(installed_gpt);
            command
                .env("TABLINQ_GPTD_BIN", installed_gptd)
                .env("TABLINQ_GATEWAY_BIN", installed_gateway)
                .arg("--state-root")
                .arg(state_root)
                .arg("gateway")
                .arg("status");
            command
        });
        let status: Value =
            serde_json::from_str(output.trim()).expect("parse packaged gateway status");
        if status["running"] == true && status["ready"] == true {
            return status;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("packaged gateway did not become healthy");
}

fn reserve_local_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral test port");
    let port = listener
        .local_addr()
        .expect("read ephemeral test port")
        .port();
    drop(listener);
    port
}

fn seed_install_config(state_root: &Path, port: u16) {
    let config_dir = state_root.join("config");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(
        config_dir.join("config.toml"),
        format!(
            "version = 2\nwarm_mode = false\nsafe_mode = false\nbrowser_target_url = 'https://chatgpt.com/'\n\n[gateway]\nhost = '127.0.0.1'\nport = {port}\n"
        ),
    )
    .expect("write seeded config.toml");
}

#[test]
fn windows_packaging_and_install_smoke_cover_packaged_layout() {
    let _guard = integration_lock();
    let repo_root = repo_root();
    let install_dir = tempdir().expect("install tempdir");
    let state_dir = tempdir().expect("state tempdir");
    let install_script = repo_root.join("scripts").join("install.ps1");
    let package_dir = tempdir().expect("package tempdir");
    let powershell = powershell();
    let package_root = if let Some(package_root) = package_root_override() {
        package_root
    } else {
        let target_dir = tempdir().expect("package target tempdir");
        let package_script = repo_root.join("scripts").join("package.ps1");
        eprintln!("packaging");
        run_success({
            let mut command = Command::new(&powershell);
            command
                .arg("-ExecutionPolicy")
                .arg("Bypass")
                .arg("-File")
                .arg(package_script)
                .arg("-Profile")
                .arg("dev")
                .arg("-OutputDir")
                .arg(package_dir.path());
            command.env("CARGO_TARGET_DIR", target_dir.path());
            command.current_dir(&repo_root);
            command
        });
        package_dir.path().join("tablinq")
    };

    eprintln!("verifying package layout");
    for relative in [
        "bin\\gpt.exe",
        "bin\\gptd.exe",
        "bin\\tablinq-gateway.exe",
        "bin\\tablinq-host.exe",
        "extension\\manifest.json",
        "browser-adapter\\src\\service.js",
        "selector-packs\\v1",
        "schemas\\v1",
        "docs\\provider-mode.md",
    ] {
        assert!(
            package_root.join(relative).exists(),
            "expected packaged path {}",
            package_root.join(relative).display()
        );
    }

    let port = reserve_local_port();
    seed_install_config(state_dir.path(), port);

    eprintln!("installing package");
    let install_output = run_success({
        let mut command = Command::new(&powershell);
        command
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(install_script)
            .arg("-PackageRoot")
            .arg(&package_root)
            .arg("-InstallRoot")
            .arg(install_dir.path())
            .arg("-StateRoot")
            .arg(state_dir.path())
            .arg("-SkipRegistry")
            .arg("-SkipPathUpdate");
        command.current_dir(&repo_root);
        command
    });
    assert!(install_output.contains("Install complete."));
    assert!(install_output.contains("Daemon:"));
    assert!(install_output.contains("Gateway:"));

    let installed_root = install_dir.path().to_path_buf();
    eprintln!("verifying install layout");
    let native_host_manifest = installed_root
        .join("native-host")
        .join("com.tablinq.bridge.chrome.json");
    let native_host: Value = serde_json::from_slice(
        &std::fs::read(&native_host_manifest).expect("read native host manifest"),
    )
    .expect("parse native host manifest");
    assert_eq!(native_host["name"], "com.tablinq.bridge");
    assert_eq!(native_host["type"], "stdio");
    assert_eq!(
        native_host["path"],
        installed_root
            .join("bin")
            .join("tablinq-host.exe")
            .display()
            .to_string()
    );
    assert!(
        native_host["allowed_origins"][0]
            .as_str()
            .expect("allowed origin")
            .starts_with("chrome-extension://")
    );

    for relative in [
        "config",
        "logs",
        "db",
        "browser-profile",
        "downloads",
        "crash-bundles",
    ] {
        assert!(
            state_dir.path().join(relative).exists(),
            "expected state path {}",
            state_dir.path().join(relative).display()
        );
    }

    let installed_gpt = installed_root.join("bin").join("gpt.exe");
    let installed_gptd = installed_root.join("bin").join("gptd.exe");
    let installed_gateway = installed_root.join("bin").join("tablinq-gateway.exe");
    eprintln!("checking packaged daemon status");
    wait_for_daemon(state_dir.path());

    eprintln!("checking packaged gateway status");
    let gateway_status = wait_for_packaged_gateway(
        &installed_gpt,
        &installed_gptd,
        &installed_gateway,
        state_dir.path(),
    );
    assert_eq!(gateway_status["running"], true);
    assert!(gateway_status["port"].as_u64().is_some());

    eprintln!("stopping packaged gateway");
    let gateway_stopped: Value = serde_json::from_str(
        run_success({
            let mut command = Command::new(&installed_gpt);
            command
                .env("TABLINQ_GPTD_BIN", &installed_gptd)
                .env("TABLINQ_GATEWAY_BIN", &installed_gateway)
                .arg("--state-root")
                .arg(state_dir.path())
                .arg("gateway")
                .arg("stop");
            command
        })
        .trim(),
    )
    .expect("parse gateway stop json");
    assert!(matches!(
        gateway_stopped["status"].as_str(),
        Some("stopped" | "not_running")
    ));

    eprintln!("stopping packaged daemon");
    let daemon_stopped: Value = serde_json::from_str(
        run_success({
            let mut command = Command::new(&installed_gpt);
            command
                .env("TABLINQ_GPTD_BIN", &installed_gptd)
                .env("TABLINQ_GATEWAY_BIN", &installed_gateway)
                .arg("--state-root")
                .arg(state_dir.path())
                .arg("daemon")
                .arg("stop");
            command
        })
        .trim(),
    )
    .expect("parse daemon stop json");
    assert!(daemon_stopped.is_object());
}
