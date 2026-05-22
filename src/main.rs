use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncReadExt;

#[tokio::main]
async fn main() {
    println!("[ok] smolprox initializing");
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::spawn(async move {
            process( socket).await;
        });
        
    }
}

async fn process(mut stream: TcpStream) {
    let mut buffer = [0; 1024];
    match stream.read(&mut buffer).await {
        Ok(n) => {
            let request = String::from_utf8_lossy(&buffer[..n]);
            println!("{}", request);
            
        }
        Err(e) => {
            eprintln!("socket error: {}", e);
        }
    }
}