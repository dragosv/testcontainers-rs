use testcontainers::{runners::AsyncRunner, GenericImage, ImageExt, ReuseDirective};

#[tokio::test]
async fn test_ryuk_behavior() {
    // 1. Normal Ryuk behavior: Container should have the session-id label
    let container = GenericImage::new("hello-world", "latest")
        .start()
        .await
        .expect("Failed to start container");
    
    let id = container.id();
    let output = std::process::Command::new("docker")
        .arg("inspect")
        .arg("--format={{json .Config.Labels}}")
        .arg(id)
        .output()
        .expect("Failed to run docker inspect");
        
    let labels_json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let labels = labels_json.as_object().unwrap();
    assert!(labels.contains_key("org.testcontainers.session-id"));

    // 2. Disabled Ryuk behavior: Container should NOT have the session-id label
    std::env::set_var("TESTCONTAINERS_RYUK_DISABLED", "true");
    let container_disabled = GenericImage::new("hello-world", "latest")
        .start()
        .await
        .expect("Failed to start container");
    
    let id_disabled = container_disabled.id();
    let output_disabled = std::process::Command::new("docker")
        .arg("inspect")
        .arg("--format={{json .Config.Labels}}")
        .arg(id_disabled)
        .output()
        .expect("Failed to run docker inspect");
        
    let labels_json_disabled: serde_json::Value = serde_json::from_slice(&output_disabled.stdout).unwrap();
    let labels_disabled = labels_json_disabled.as_object().unwrap();
    assert!(!labels_disabled.contains_key("org.testcontainers.session-id"));
    std::env::remove_var("TESTCONTAINERS_RYUK_DISABLED");
}

#[cfg(feature = "reusable-containers")]
#[tokio::test]
async fn test_ryuk_ignores_always_reusable() {
    let container = GenericImage::new("hello-world", "latest")
        .with_reuse(ReuseDirective::Always)
        .start()
        .await
        .expect("Failed to start container");
    
    let id = container.id();
    
    let output = std::process::Command::new("docker")
        .arg("inspect")
        .arg("--format={{json .Config.Labels}}")
        .arg(id)
        .output()
        .expect("Failed to run docker inspect");
        
    let labels_json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let labels = labels_json.as_object().unwrap();
    
    assert!(!labels.contains_key("org.testcontainers.session-id"));
}
