use super::process::TestChild;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

pub fn spawn_fake_chatgpt() -> (TestChild, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("allocate tcp port");
    let port = listener.local_addr().expect("resolve local address").port();
    drop(listener);

    let server_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packages")
        .join("fake-chatgpt")
        .join("server.mjs");
    let mut command = Command::new("node");
    command
        .arg(server_path)
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = TestChild::spawn(command);

    let url = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, url);
        }
        thread::sleep(StdDuration::from_millis(100));
    }

    panic!("fake-chatgpt did not become ready on port {port}");
}
