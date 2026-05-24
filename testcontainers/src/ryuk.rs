//! Ryuk resource reaper support.
//!
//! Ryuk is a sidecar container (`testcontainers/ryuk`) that automatically removes Docker
//! resources (containers, networks, volumes) associated with the current test session when the
//! test process exits — even on panic or SIGKILL.
//!
//! **How it works:**
//! 1. A Ryuk container is started once per process and given a bind-mount of the Docker socket.
//! 2. A TCP connection is opened to the Ryuk port and a label filter is registered:
//!    `label=org.testcontainers.session-id=<session-id>`.
//! 3. Every container (and network) started by this process is tagged with that label.
//! 4. When the TCP connection closes (process exit, drop, panic), Ryuk removes all matching
//!    resources after its configured reconnect timeout.
//!
//! **Disabling Ryuk:** set `TESTCONTAINERS_RYUK_DISABLED=true` (or `=1`).
//! **Overriding the image:** set `TESTCONTAINERS_RYUK_IMAGE=<image:tag>`.

use std::sync::Arc;

use bollard::models::{ContainerCreateBody, HostConfig, Mount as BollardMount, MountTypeEnum};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    sync::OnceCell,
};

use crate::core::{
    client::{Client, ClientError},
    error::TestcontainersError,
};

/// Docker label key used to associate resources with a test session.
pub(crate) const SESSION_LABEL_KEY: &str = "org.testcontainers.session-id";

const RYUK_IMAGE: &str = "testcontainers/ryuk";
const RYUK_DEFAULT_TAG: &str = "0.11.0";
const RYUK_INTERNAL_PORT: u16 = 8080;

/// The session identifier, generated once per process.
///
/// All containers and networks started in this process are tagged with this value so that
/// Ryuk can clean them up.
pub(crate) static SESSION_ID: std::sync::LazyLock<String> =
    std::sync::LazyLock::new(|| ferroid::id::ULID::now().to_string());

/// Returns the current session ID as a `&str`.
pub(crate) fn session_id() -> &'static str {
    &SESSION_ID
}

/// Returns `true` when Ryuk is disabled via `TESTCONTAINERS_RYUK_DISABLED`.
pub(crate) fn is_disabled() -> bool {
    std::env::var("TESTCONTAINERS_RYUK_DISABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false)
}

fn ryuk_image() -> String {
    std::env::var("TESTCONTAINERS_RYUK_IMAGE")
        .unwrap_or_else(|_| format!("{RYUK_IMAGE}:{RYUK_DEFAULT_TAG}"))
}

/// Extracts the socket path from a `unix://` URI.
fn docker_socket_path(docker_host: &str) -> Option<String> {
    docker_host.strip_prefix("unix://").map(str::to_string)
}

// ---------------------------------------------------------------------------
// Global singleton
// ---------------------------------------------------------------------------

static RYUK: OnceCell<Arc<RyukHandle>> = OnceCell::const_new();

/// Ensures the Ryuk reaper is running and connected.
///
/// This is a no-op when `TESTCONTAINERS_RYUK_DISABLED=true`.
/// It is safe to call from multiple tasks concurrently; Ryuk will only be
/// started once per process.
pub(crate) async fn ensure_ryuk_running(client: &Arc<Client>) -> Result<(), TestcontainersError> {
    if is_disabled() {
        return Ok(());
    }
    RYUK.get_or_try_init(|| start_ryuk(client.clone()))
        .await
        .map(|_| ())
}

// ---------------------------------------------------------------------------
// RyukHandle
// ---------------------------------------------------------------------------

/// Owns the Ryuk container and the open TCP connection.
///
/// Dropping this handle closes the TCP connection, which signals Ryuk to begin
/// cleaning up all resources labelled with [`SESSION_LABEL_KEY`].
struct RyukHandle {
    container_id: String,
    client: Arc<Client>,
    /// The open TCP stream to Ryuk. Must remain open for the lifetime of the
    /// process (or until we intentionally close it).
    _stream: tokio::sync::Mutex<BufReader<TcpStream>>,
}

impl Drop for RyukHandle {
    fn drop(&mut self) {
        let client = self.client.clone();
        let id = self.container_id.clone();
        crate::core::async_drop::async_drop(async move {
            log::debug!("Removing Ryuk container {id}");
            let _ = client.rm(&id).await;
        });
    }
}

// ---------------------------------------------------------------------------
// Startup
// ---------------------------------------------------------------------------

async fn start_ryuk(client: Arc<Client>) -> Result<Arc<RyukHandle>, TestcontainersError> {
    let image = ryuk_image();
    log::debug!("Starting Ryuk resource reaper ({image})");

    let docker_host = client.config.docker_host();
    let socket_mounts: Option<Vec<BollardMount>> =
        docker_socket_path(&docker_host).map(|socket_path| {
            vec![BollardMount {
                target: Some("/var/run/docker.sock".to_string()),
                source: Some(socket_path),
                typ: Some(MountTypeEnum::BIND),
                read_only: Some(false),
                ..Default::default()
            }]
        });

    // When the Docker host is not a Unix socket (e.g. TCP), pass DOCKER_HOST to Ryuk so it
    // can reach the daemon without a socket mount.
    let env: Option<Vec<String>> = if docker_host.starts_with("unix://") {
        None
    } else {
        Some(vec![format!("DOCKER_HOST={docker_host}")])
    };

    let container_config = ContainerCreateBody {
        image: Some(image.clone()),
        env,
        exposed_ports: Some(vec![format!(
            "{}",
            crate::core::ports::ContainerPort::Tcp(RYUK_INTERNAL_PORT)
        )]),
        host_config: Some(HostConfig {
            privileged: Some(true),
            publish_all_ports: Some(true),
            mounts: socket_mounts,
            ..Default::default()
        }),
        ..Default::default()
    };

    // Create the container, pulling the image if it is not yet present.
    let container_id = match client
        .create_container(None, container_config.clone())
        .await
    {
        Err(ClientError::CreateContainer(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        })) => {
            client.pull_image(&image, None).await?;
            client.create_container(None, container_config).await
        }
        other => other,
    }?;

    client.start_container(&container_id).await?;

    // Resolve the mapped host port for Ryuk's TCP listener.
    let ports = client.ports(&container_id).await?;
    let host_port = ports
        .map_to_host_port_ipv4(crate::core::ports::ContainerPort::Tcp(RYUK_INTERNAL_PORT))
        .ok_or_else(|| {
            TestcontainersError::other(format!(
                "Ryuk container {container_id} did not expose port {RYUK_INTERNAL_PORT}"
            ))
        })?;

    // Connect to Ryuk using the resolved Docker host so remote daemons and
    // in-container clients reach the published port correctly.
    let docker_host = client.docker_hostname().await?;
    let stream = match docker_host {
        url::Host::Domain(domain) => TcpStream::connect((domain.as_str(), host_port)).await,
        url::Host::Ipv4(address) => TcpStream::connect((address, host_port)).await,
        url::Host::Ipv6(address) => TcpStream::connect((address, host_port)).await,
    }
    .map_err(|e| TestcontainersError::other(format!("Cannot connect to Ryuk: {e}")))?;

    let mut buf_reader = BufReader::new(stream);

    let filter = format!("label={}={}\n", SESSION_LABEL_KEY, session_id());
    buf_reader
        .get_mut()
        .write_all(filter.as_bytes())
        .await
        .map_err(|e| TestcontainersError::other(format!("Cannot send filter to Ryuk: {e}")))?;

    let mut ack = String::new();
    buf_reader
        .read_line(&mut ack)
        .await
        .map_err(|e| TestcontainersError::other(format!("Cannot read ACK from Ryuk: {e}")))?;

    if ack.trim() != "ACK" {
        return Err(TestcontainersError::other(format!(
            "Unexpected response from Ryuk: {ack:?}"
        )));
    }

    log::debug!(
        "Ryuk ready (container: {container_id}, port: {host_port}, session: {})",
        session_id()
    );

    Ok(Arc::new(RyukHandle {
        container_id,
        client,
        _stream: tokio::sync::Mutex::new(buf_reader),
    }))
}
