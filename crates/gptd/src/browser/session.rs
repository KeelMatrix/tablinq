use super::adapter_process::AdapterProcess;
use super::protocol::AdapterRequest;
use super::recovery::{idle_snapshot, normalize_snapshot};
use super::{BrowserManager, ExecutionErrorMetadata, execution_error};
use crate::config::AppConfig;
use crate::logging::JsonLogger;
use anyhow::Result;
use serde_json::json;
use shared_protocol::{
    BrowserConnectionMode, BrowserOpenRequest, BrowserStatusSnapshot, ErrorCode,
};

impl BrowserManager {
    pub fn open(&mut self, request: BrowserOpenRequest) -> Result<BrowserStatusSnapshot> {
        self.open_impl(request, false)
    }

    pub fn open_fresh_conversation(
        &mut self,
        request: BrowserOpenRequest,
    ) -> Result<BrowserStatusSnapshot> {
        self.open_impl(request, true)
    }

    fn open_impl(
        &mut self,
        request: BrowserOpenRequest,
        force_reload: bool,
    ) -> Result<BrowserStatusSnapshot> {
        self.ensure_process()?;

        let target_url = request
            .target_url
            .unwrap_or_else(|| self.config.browser_target_url().to_owned());
        let browser_path = request
            .browser_path
            .or_else(|| self.config.browser_path().map(ToOwned::to_owned));
        let headless = request.headless.unwrap_or(false);

        let request_browser_path = browser_path.clone();
        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Open {
                target_url,
                browser_path,
                headless,
                force_reload,
            }) {
            Ok(snapshot) => {
                let snapshot = normalize_snapshot(&self.config, snapshot);
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
            Err(err) => {
                let mapped_error = attach_endpoint_unavailable_error(&self.config, &err);
                self.teardown_process();
                self.last_snapshot = idle_snapshot(
                    &self.config,
                    Some(self.current_target()),
                    request_browser_path,
                );
                Err(mapped_error.unwrap_or(err))
            }
        }
    }

    pub fn close(&mut self) -> Result<BrowserStatusSnapshot> {
        let attach_primary = matches!(
            self.config.browser_connection_mode(),
            BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
        );
        if let Some(process) = self.process.as_mut() {
            let _ = process.request(AdapterRequest::Close);
            let _ = process.request(AdapterRequest::Shutdown);
        }
        self.teardown_process();
        let mut snapshot = idle_snapshot(
            &self.config,
            Some(self.last_snapshot.target_url.clone()),
            self.last_snapshot.browser_path.clone(),
        );
        if attach_primary {
            let browser_label = if self.config.browser_channel().eq_ignore_ascii_case("msedge")
                || self.config.browser_channel().eq_ignore_ascii_case("edge")
            {
                "Edge"
            } else {
                "Chrome"
            };
            snapshot.action = Some("detached".to_owned());
            snapshot.browser_process_closed = Some(false);
            snapshot.reason = Some(format!(
                "attach-first uses a user-owned {browser_label} profile; Tablinq only disconnects from CDP"
            ));
        } else {
            snapshot.action = Some("closed".to_owned());
            snapshot.browser_process_closed = Some(true);
            snapshot.reason = Some("managed browser process closed by Tablinq".to_owned());
        }
        self.last_snapshot = snapshot.clone();
        Ok(snapshot)
    }

    pub fn reload_target_page(&mut self) -> Result<BrowserStatusSnapshot> {
        self.ensure_process()?;

        let target_url = self.current_target();
        let browser_path = self.last_snapshot.browser_path.clone();
        let request_browser_path = browser_path.clone();
        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Open {
                target_url,
                browser_path,
                headless: false,
                force_reload: true,
            }) {
            Ok(snapshot) => {
                let snapshot = normalize_snapshot(&self.config, snapshot);
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
            Err(err) => {
                self.teardown_process();
                self.last_snapshot = idle_snapshot(
                    &self.config,
                    Some(self.current_target()),
                    request_browser_path,
                );
                Err(err)
            }
        }
    }

    pub fn health(&mut self) -> Result<BrowserStatusSnapshot> {
        if self.process.is_none() {
            return Ok(self.last_snapshot.clone());
        }

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Health)
        {
            Ok(snapshot) => {
                let snapshot = normalize_snapshot(&self.config, snapshot);
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
            Err(_) => {
                self.teardown_process();
                let snapshot = idle_snapshot(
                    &self.config,
                    Some(self.last_snapshot.target_url.clone()),
                    self.last_snapshot.browser_path.clone(),
                );
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
        }
    }

    pub(super) fn ensure_process(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        self.process = Some(AdapterProcess::spawn(&self.config)?);
        Ok(())
    }

    pub(super) fn teardown_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            self.log_process_event(
                "browser adapter teardown starting",
                json!({
                    "teardown_in_progress": true,
                    "target_url": self.last_snapshot.target_url,
                }),
            );
            process.terminate();
            self.log_process_event(
                "browser adapter teardown finished",
                json!({
                    "teardown_in_progress": false,
                    "target_url": self.last_snapshot.target_url,
                }),
            );
        }
    }

    pub(super) fn relaunch_for_recovery(&mut self) -> Result<()> {
        self.log_process_event(
            "browser adapter recovery relaunch starting",
            json!({
                "teardown_in_progress": self.process.is_some(),
                "target_url": self.last_snapshot.target_url,
            }),
        );
        self.teardown_process();
        self.ensure_process()?;
        self.log_process_event(
            "browser adapter recovery relaunch finished",
            json!({
                "teardown_in_progress": false,
                "adapter_running": self.process.is_some(),
                "target_url": self.last_snapshot.target_url,
            }),
        );
        Ok(())
    }

    pub(super) fn current_target(&self) -> String {
        self.last_snapshot.target_url.clone()
    }

    fn log_process_event(&self, message: &str, fields: serde_json::Value) {
        if let Ok(logger) = JsonLogger::new(&self.config.paths.log_path) {
            let _ = logger.info(message, fields);
        }
    }
}

fn attach_endpoint_unavailable_error(
    config: &AppConfig,
    err: &anyhow::Error,
) -> Option<anyhow::Error> {
    if !matches!(
        config.browser_connection_mode(),
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    ) {
        return None;
    }
    let message = format!("{err:#}");
    if !message.contains("cdp-connect-failed") && !message.contains("ECONNREFUSED") {
        return None;
    }
    let browser_label = if config.browser_channel().eq_ignore_ascii_case("msedge")
        || config.browser_channel().eq_ignore_ascii_case("edge")
    {
        "Edge"
    } else {
        "Chrome"
    };
    Some(execution_error(ExecutionErrorMetadata::new(
        ErrorCode::AttachEndpointUnavailable,
        format!(
            "{browser_label} attach debug endpoint is not reachable at {}. Run `gpt browser start-debug`. Then run `gpt browser attach`.",
            config.browser_debug_endpoint()
        ),
        None,
    )))
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        self.teardown_process();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::execution_error_metadata;
    use tempfile::tempdir;

    #[test]
    fn attach_endpoint_error_is_typed_and_actionable() {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        config.file.browser_channel = Some("chrome".to_owned());
        config.file.browser_debug_endpoint = Some("http://127.0.0.1:9222".to_owned());
        let raw = anyhow::anyhow!(
            "cdp-connect-failed: browserType.connectOverCDP: connect ECONNREFUSED 127.0.0.1:9222"
        );

        let err = attach_endpoint_unavailable_error(&config, &raw).unwrap();
        let metadata = execution_error_metadata(&err).unwrap();

        assert_eq!(metadata.error_code, ErrorCode::AttachEndpointUnavailable);
        assert_eq!(
            metadata.message,
            "Chrome attach debug endpoint is not reachable at http://127.0.0.1:9222. Run `gpt browser start-debug`. Then run `gpt browser attach`."
        );
        assert!(!metadata.message.contains("browserType.connectOverCDP"));
    }
}
