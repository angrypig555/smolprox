use tokio::net::{TcpListener, TcpStream};
use tokio::io::AsyncReadExt;
use log_overflow::{log, log_init, Severity};
use tokio::io::copy;
use tokio::io::AsyncWriteExt;
use std::io::{Error, ErrorKind, Result};
use clap::Parser;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
struct Args {
    /// Port number
    #[arg(short, long, default_value_t = 8080)]
    port: u16,

    /// Turn off logging
    #[arg(short, long)]
    nolog: bool,

    /// Whitelist an IP
    #[arg(short, long, default_value_t = String::from("no"))]
    whitelist: String,

    /// Username for SOCKS5 Proxy (Optional)
    #[arg(long)]
    username: Option<String>,

    /// Password for SOCKS5 Proxy (Optional)
    #[arg(long)]
    password: Option<String>,
}
#[tokio::main]
async fn main() {
    let args = Args::parse();
    log_init("smolprox", "~/.cache/smolprox", !args.nolog);
    println!("[ok] smolprox running");
    println!("see logs for more info");
    
    log(Severity::DEBUG, "smolprox started");
    if args.password.is_some() && args.username.is_none() {
        panic!("password was defined but username was not");
    } else if args.password.is_none() && args.username.is_some() {
        panic!("username was defined but password was not");
    }
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    loop {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        let client_ip = remote_addr.ip().to_string();
        if args.whitelist != "no" && client_ip != args.whitelist {
            log(Severity::WARNING, "Not whitelisted client tried to connect, silently dropped connection");
            continue;
        }
        let args_clone = args.clone();
        tokio::spawn(async move {
            if let Err(e) = process( socket, args_clone).await {
                log(Severity::CRITICAL, &format!("Proxy error: {}", e));
            }
        });
        
    }
}

async fn process_socks5(mut stream: TcpStream, args: Args) -> Result<()> {
    let mut initial_handshake = [0; 1024];
    stream.read(&mut initial_handshake).await?;
    let version = initial_handshake[0];
    let num_methods = initial_handshake[1] as usize;
    let methods = &initial_handshake[2..2 + num_methods];
    if version != 0x05 {
        return Err(Error::new(ErrorKind::InvalidData, "not socks5"));
    }
    if methods.contains(&0x02) && args.username.is_some() {
        let response = [0x05, 0x02];
        stream.write_all(&response).await?;
        let mut sub_header = [0x00; 0x02];
        let auth_response = stream.read_exact(&mut sub_header).await?;
        let user_len = sub_header[1] as usize;
        let mut user_buf = vec![0x00; user_len];
        stream.read_exact(&mut user_buf).await?;
        let username = String::from_utf8_lossy(&user_buf);
        let pass_len = stream.read_u8().await? as usize;
        let mut pass_buf = vec![0x00; pass_len];
        stream.read_exact(&mut pass_buf).await?;
        let password = String::from_utf8_lossy(&pass_buf);
        if username == *args.username.as_ref().unwrap() && password == *args.password.as_ref().unwrap() {
            let succes_response = [0x01, 0x00];
            stream.write_all(&succes_response).await?;
            log(Severity::INFO, "Authentication succesful");
        } else {
            let fail_response = [0x01, 0x01];
            stream.write_all(&fail_response).await?;
            return Err(Error::new(ErrorKind::PermissionDenied, "Client authentication failed"))
        }
    } else if methods.contains(&0x00) && args.username.is_none() {
        let response = [0x05, 0x00];
        stream.write_all(&response).await?;
    } else  {
        let response = [0x05, 0xFF];    
        stream.write_all(&response).await?;
        return Err(Error::new(ErrorKind::InvalidData, "Client does not have the required authentication method"))
    }
    
    let mut header = [0; 4];
    stream.read_exact(&mut header).await?;
    let atyp = header[3];
    let target: String = match atyp {
        0x01 => {
            let mut buf = [0; 6];
            stream.read_exact(&mut buf).await?;
            let ip = Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3]);
            let port = u16::from_be_bytes([buf[4], buf[5]]);
            format!("{}:{}", ip, port)
        }
        0x03 => {
            let len = stream.read_u8().await? as usize;
            let mut buf = vec![0; len + 2];
            stream.read_exact(&mut buf).await?;
            let domain = String::from_utf8_lossy(&buf[..len]);
            let port = u16::from_be_bytes([buf[len], buf[len + 1]]);
            format!("{}:{}", domain, port)
        }
        0x04 => {
            let mut buf = [0; 18];
            stream.read_exact(&mut buf).await?;
            let mut ipv6_bytes = [0; 16];
            ipv6_bytes.copy_from_slice(&buf[..16]);
            let ip = Ipv6Addr::from(ipv6_bytes);
            //let ip = Ipv6Addr::new(buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7], buf[8], buf[9], buf[10], buf[11], buf[12], buf[13], buf[14], buf[15], buf[16]);
            let port = u16::from_be_bytes([buf[17], buf[18]]);
            format!("{}:{}", ip, port)
        }
        _ => return Err(Error::new(ErrorKind::InvalidData, "Invalid Target address for SOCKS5"))
    };
    let mut target_stream = match TcpStream::connect(target).await {
        Ok(stream) => stream,
        Err(e) => {
            let response = [0x05, 0x04, 0x00];
            stream.write_all(&response).await?;
            return Err(Error::new(ErrorKind::HostUnreachable, format!("Could not reach host: {}", e)))
        }
    };
    log(Severity::DEBUG, "Succesfully established connection with target, tunnel handoff");
    let success_response = [0x05, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    stream.write_all(&success_response).await?;
    let (mut client_reader, mut client_writer) = stream.into_split();
    let (mut target_reader, mut target_writer) = target_stream.into_split();
    let client_to_target = tokio::spawn(async move {
        let _ = copy(&mut client_reader, &mut target_writer).await;
    });
    let _ = copy(&mut target_reader, &mut client_writer).await;
    let _ = client_to_target.await;
    Ok(())
}

async fn process(mut stream: TcpStream, args: Args) -> Result<()>{
    let mut peek_buf = [0; 1];
    let _ = stream.peek(&mut peek_buf).await?;
    let first_byte = peek_buf[0];
    if first_byte == 0x05 {
        log(Severity::INFO, "detected socks5");
        return process_socks5(stream, args).await;
    }

    let mut buffer = [0; 1024];
    let n = stream.read(&mut buffer).await?;
    let request = String::from_utf8_lossy(&buffer[..n]);
    let request_line = request.lines().next().unwrap_or("");
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let response = format!(
            "HTTP/1.1 400 Bad Request\r\nServer: smolprox\r\nContent-Type: text/plain\r\nContent-Length: 11\r\nConnection: close\r\n\r\nBad request",
        );
        stream.write_all(response.as_bytes()).await?;
        log(Severity::WARNING,"less than 2 parts in header, 400");
        return Err(Error::new(ErrorKind::InvalidData, "less than 2 parts in header, 400 bad request"));
    }
    let method = parts[0];
    if method != "CONNECT" {
        let response = "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nServer: smolprox\r\nAllow: CONNECT\r\nConnection: close\r\n\r\n";
        stream.write_all(response.as_bytes()).await?;
        log(Severity::WARNING, "Client attempted to use unsupported method");
        return Err(Error::new(ErrorKind::InvalidData, "Unsupported method"));
    }
    let target = parts[1];
    log(Severity::INFO, &format!("Client connecting to {}", target));
    let mut target_stream = match TcpStream::connect(target).await {
        Ok(stream) => stream,
        Err(e) => {
            log(Severity::CRITICAL, &format!("{} is unreachable: {}", target, e));
            let response = "HTTP/1.1 502 Bad Gateway\r\nServer: smolprox\r\nContent-Type: text/plain\r\nContent-Length: 32\r\nConnection: close\r\n\r\nProxy Error: Target unreachable";
            let _ = stream.write_all(response.as_bytes()).await;
            return Err(e);
        }
    };
    log(Severity::DEBUG, "Succesfully established connection with target, tunnel handoff");
    let connected_response = "HTTP/1.1 200 Connection Established\r\nProxy-Agent: smolprox/1.0\r\n\r\n";
    stream.write_all(connected_response.as_bytes()).await?;
    let (mut client_reader, mut client_writer) = stream.into_split();
    let (mut target_reader, mut target_writer) = target_stream.into_split();
    let client_to_target = tokio::spawn(async move {
        let _ = copy(&mut client_reader, &mut target_writer).await;
    });
    let _ = copy(&mut target_reader, &mut client_writer).await;
    let _ = client_to_target.await;
    Ok(())    
}