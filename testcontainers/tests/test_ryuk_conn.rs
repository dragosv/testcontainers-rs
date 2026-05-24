use testcontainers::{core::client::Client, runners::AsyncRunner};

#[tokio::test]
async fn test_ryuk_conn() {
    let client = Client::lazy_client().await.unwrap();
    let host = client.docker_hostname().await.unwrap();
    println!("DOCKER HOST: {:?}", host);
}
