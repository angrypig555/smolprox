use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncReadExt;
use log_overflow::{log, log_init, Severity};
use tokio::io::copy;
use tokio::io::AsyncWriteExt;
use std::io::{Error, ErrorKind, Result};


#[tokio::main]
async fn main() {
    log_init("smolprox", "~/.cache/smolprox", true);
    println!("[ok] smolprox initializing");
    log(Severity::DEBUG, "smolprox started");
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            process( socket).await;
        });
        
    }
}

async fn process(mut stream: TcpStream) -> Result<()>{
    let mut buffer = [0; 1024];
    match stream.read(&mut buffer).await {
        Ok(n) => {
            let request = String::from_utf8_lossy(&buffer[..n]);
            let request_line = request.lines().next().unwrap_or("");
            let parts: Vec<&str> = request_line.split_whitespace().collect();
            if parts.len() < 2 {
                let response = format!(
                    "HTTP/1.1 400 Bad Request\r\nServer: smolprox\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad request",
                );
                stream.write_all(response.as_bytes()).await.expect("Failed to write to socket");
                log(Severity::WARNING,"less than 2 parts in header, 400");
            }
            let method = parts[0];
            if method != "CONNECT" {
                let response = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nServer: smolprox\r\nAllow: CONNECT\r\nConnection: close\r\n\r\n";
                stream.write_all(response.as_bytes()).await.expect("Failed to write to socket");
                log(Severity::WARNING, "Client attempted to use unsupported method");
            }
            let target = parts[1];
            log(Severity::INFO, &format!("Client connecting to {}", target));
            let target_stream = TcpStream::connect(target).await.expect("Failed to connect to target");
            let connected_response = "HTTP/1.1 200 Connection Established\r\nProxy-Agent: smolprox/1.0\r\n\r\n";
            stream.write_all(connected_response.as_bytes()).await.expect("Failed to write to socket");
            let (mut client_reader, mut client_writer) = stream.into_split();
            let (mut target_reader, mut target_writer) = target_stream.into_split();
            let client_to_target = tokio::spawn(async move {
                let _ = copy(&mut client_reader, &mut target_writer).await;
            });
            let _ = copy(&mut target_reader, &mut client_writer).await;
            let _ = client_to_target.await;
        }
        Err(e) => {
            eprintln!("socket error: {}", e);
        }
    }
}