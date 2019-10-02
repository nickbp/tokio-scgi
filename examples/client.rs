#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use std::env;
use std::io::{Error, ErrorKind};
use std::net::ToSocketAddrs;
use std::path::Path;
use tokio::net::{TcpStream, UnixStream};
use tokio::prelude::*;
use tokio_codec::Framed;
use tokio_scgi::client::{SCGICodec, SCGIRequest};

fn syntax() -> Error {
    println!(
        "Syntax: {} </path/to/unix.sock or tcp-host:1234>",
        env::args().nth(0).unwrap()
    );
    Error::new(ErrorKind::InvalidInput, "Missing required argument")
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if env::args().len() <= 1 {
        return Err(syntax());
    }
    let endpoint = env::args().nth(1).unwrap();
    if endpoint.starts_with('-') {
        // Probably a commandline argument like '-h'/'--help', avoid parsing as a hostname
        return Err(syntax());
    }
    if endpoint.contains('/') {
        // Probably a path to a file, assume the argument is a unix socket
        let addr = Path::new(&endpoint);
        println!("Connecting to {}", addr.display());
        let mut conn = UnixStream::connect(&addr).await?;
        run_client(&mut conn).await
    } else {
        // Probably a TCP endpoint, try to resolve it in case it's a hostname
        let addr = endpoint
            .to_socket_addrs()
            .expect(format!("Invalid TCP endpoint '{}'", endpoint).as_str())
            .next()
            .unwrap();
        println!("Connecting to {}", addr);
        let mut conn = TcpStream::connect(&addr).await?;
        run_client(&mut conn).await
    }
}

/// Runs the client: Sends a request and prints the responses via the provided UDS or TCP connection.
async fn run_client<C>(conn: &mut C) -> Result<(), Error>
where
    C: AsyncRead + AsyncWrite + std::marker::Send + std::marker::Unpin + std::fmt::Debug,
{
    let (mut tx_scgi, mut rx_scgi) = Framed::new(conn, SCGICodec::new()).split();

    // Send request
    tx_scgi.send(build_request()).await?;

    // Consume response(s): loop until error or empty data returned
    loop {
        match rx_scgi.into_future().await {
            (None, new_rx) => {
                // SCGI response not ready: loop for more rx data
                // Shouldn't happen for response data, but this is how it would work...
                println!("Response data is incomplete, resuming read");
                rx_scgi = new_rx;
            }
            (Some(Err(e)), _new_rx) => {
                // RX error: return error and abort
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("Error when waiting for response: {}", e),
                ));
            }
            (Some(Ok(response)), new_rx) => {
                // Got SCGI response: if empty, treat as end of response.
                if response.len() == 0 {
                    return Ok(());
                }
                // Otherwise 'handle' by printing content, then resume read for more
                rx_scgi = new_rx;
                match String::from_utf8(response.to_vec()) {
                    Ok(s) => println!("Got {} bytes:\n{}", response.len(), s),
                    Err(e) => println!(
                        "{} byte response is not UTF8 ({}):\n{:?}",
                        response.len(),
                        e,
                        response
                    ),
                }
            }
        }
    }
}

fn build_request() -> SCGIRequest {
    let content_str = b"{\"description\": \"my name is also bort <><><>\"}";
    let mut content = BytesMut::with_capacity(content_str.len());
    content.put(content_str.to_vec());

    let mut headers = Vec::new();
    headers.push(("Content-Length".to_string(), content_str.len().to_string()));
    headers.push(("SCGI".to_string(), "1".to_string()));
    headers.push(("Content-Type".to_string(), "application/json".to_string()));
    headers.push(("X-Username".to_string(), "bort".to_string()));

    SCGIRequest::Request(headers, content)
}
