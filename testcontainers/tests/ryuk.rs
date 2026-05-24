#![cfg(feature = "ryuk")]

use std::process::Command;

use testcontainers::{runners::AsyncRunner, GenericImage};

#[cfg(feature = "reusable-containers")]
use testcontainers::{ImageExt, ReuseDirective};

const SESSION_LABEL_KEY: &str = "org.testcontainers.session-id";

struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = self.previous.take() {
            std::env::set_var(self.key, value);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn has_session_label(container_id: &str) -> bool {
    let output = Command::new("docker")
        .arg("inspect")
        .arg("--format={{json .Config.Labels}}")
        .arg(container_id)
        .output()
        .expect("Failed to run docker inspect");

    assert!(
        output.status.success(),
        "docker inspect failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let labels_json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("Invalid labels JSON from docker inspect");

    labels_json
        .as_object()
        .map(|labels| labels.contains_key(SESSION_LABEL_KEY))
        .unwrap_or(false)
}

#[tokio::test]
async fn test_ryuk_behavior() {
    // Normal Ryuk behavior: container should carry the session-id label.
    let container = GenericImage::new("hello-world", "latest")
        .start()
        .await
        .expect("Failed to start container");
    assert!(has_session_label(container.id()));

    // Disabled Ryuk behavior: container should not carry the session-id label.
    let _guard = EnvVarGuard::set("TESTCONTAINERS_RYUK_DISABLED", "true");
    let container_disabled = GenericImage::new("hello-world", "latest")
        .start()
        .await
        .expect("Failed to start container");
    assert!(!has_session_label(container_disabled.id()));
}

#[cfg(feature = "reusable-containers")]
#[tokio::test]
async fn test_ryuk_ignores_always_reusable() {
    let container = GenericImage::new("hello-world", "latest")
        .with_reuse(ReuseDirective::Always)
        .start()
        .await
        .expect("Failed to start container");

    assert!(!has_session_label(container.id()));
}
