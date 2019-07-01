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
use tokio_sync::oneshot;
use tokio_sync::oneshot::Sender;

fn syntax() -> Error {
    println!(
        "Syntax: {} </path/to/unix.sock or tcp-host:1234>",
        env::args().nth(0).unwrap()
    );
    Error::new(ErrorKind::InvalidInput, "Missing required argument")
}

fn main() -> Result<(), Error> {
    if env::args().len() <= 1 {
        return Err(syntax());
    }
    let endpoint = env::args().nth(1).unwrap();
    if endpoint.starts_with('-') {
        // Probably a commandline argument like '-h'/'--help', avoid parsing as a hostname
        return Err(syntax());
    }

    // Create a channel which will be provided the response by the async callbacks once it's arrived.
    // Meanwhile we wait for the response on this end of things.
    let (sender, receiver) = oneshot::channel::<Option<BytesMut>>();
    if endpoint.contains('/') {
        // Probably a path to a file, assume the argument is a unix socket
        let addr = Path::new(&endpoint);
        println!("Connecting to {}", addr.display());
        connect(UnixStream::connect(&addr), sender);
    } else {
        // Probably a TCP endpoint, try to resolve it in case it's a hostname
        let addr = endpoint
            .to_socket_addrs()
            .expect(format!("Invalid TCP endpoint '{}'", endpoint).as_str())
            .next()
            .unwrap();
        println!("Connecting to {}", addr);
        connect(TcpStream::connect(&addr), sender);
    }

    // Wait for the callbacks to get the response and provide it to the channel.
    match receiver.wait() {
        Ok(Some(response)) => {
            match String::from_utf8(response.to_vec()) {
                Ok(s) => println!("Got {} bytes:\n{}", response.len(), s),
                Err(e) => println!("{} byte response is not UTF8 ({}):\n{:?}", response.len(), e, response)
            }
            Ok(())
        }
        Ok(None) => Err(Error::new(ErrorKind::Other, "No response received")),
        Err(e) => Err(Error::new(
            ErrorKind::Other,
            format!("Error when waiting for query result: {}", e),
        )),
    }
}

/// Schedules a `send()` call to be triggered after the connection is made.
fn connect<C, F>(connect_future: F, output: Sender<Option<BytesMut>>)
where
    C: AsyncRead + AsyncWrite + std::marker::Send + std::fmt::Debug + 'static,
    F: Future<Item = C, Error = Error> + std::marker::Send + 'static,
{
    let cb = connect_future
        .map_err(|e| {
            println!("connect error = {:?}", e);
            //output.send(None);
        })
        .and_then(move |conn| {
            send(conn, output);
            Ok(())
        });
    // The first one in the chain must use tokio::run.
    // tokio::spawn can only be called inside the runtime.
    tokio::run(cb);
}

/// Schedules sending the request payload. Once the send is complete, `recv()` is called for
/// handling the response.
fn send<C>(conn: C, output: Sender<Option<BytesMut>>)
where
    C: AsyncRead + AsyncWrite + std::marker::Send + std::fmt::Debug + 'static,
{
    let (tx_scgi, rx_scgi) = Framed::new(conn, SCGICodec::new()).split();
    let cb = tx_scgi
        .send(build_request())
        .map_err(|e| {
            println!("send error = {:?}", e);
            //output.send(None);
        })
        .and_then(move |_| {
            recv(rx_scgi, output);
            Ok(())
        });
    tokio::spawn(cb);
}

/// Schedules receiving the response. In this demo the response is printed to the console.
fn recv<R>(rx_scgi: R, output: Sender<Option<BytesMut>>)
where
    R: Stream<Item = BytesMut, Error = Error> + std::marker::Send + std::fmt::Debug + 'static,
{
    // TODO repeatedly recv until disconnected by server?
    let cb = rx_scgi
        .into_future()
        .map_err(|e| {
            println!("recv error = {:?}", e);
            //output.send(None);
        })
        .and_then(move |(response, _stream)| {
            if let Err(_response) = output.send(response) {
                println!("Failed to send response");
            }
            Ok(())
        });
    tokio::spawn(cb);
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
