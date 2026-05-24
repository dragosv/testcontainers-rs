    // Connect to Ryuk using the resolved Docker host so remote daemons and
    // in-container clients reach the published port correctly.
    let docker_host = client.docker_hostname().await?;
    let mut attempt = 0;
    let (buf_reader, ack) = loop {
        let stream_res = match &docker_host {
            url::Host::Domain(domain) => TcpStream::connect((domain.as_str(), host_port)).await,
            url::Host::Ipv4(address) => TcpStream::connect((*address, host_port)).await,
            url::Host::Ipv6(address) => TcpStream::connect((*address, host_port)).await,
        };

        if let Ok(mut stream) = stream_res {
            let mut buf_reader = BufReader::new(stream);
            let filter = format!("label={}={}\n", SESSION_LABEL_KEY, session_id());
            
            if buf_reader.get_mut().write_all(filter.as_bytes()).await.is_ok() {
                let mut ack = String::new();
                if buf_reader.read_line(&mut ack).await.is_ok() {
                    if ack.trim() == "ACK" {
                        break (buf_reader, ack);
                    }
                }
            }
        }
        
        attempt += 1;
        if attempt >= 10 {
            return Err(TestcontainersError::other("Cannot connect to Ryuk and receive ACK"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    };
