#![deny(warnings, rust_2018_idioms)]

fn main() -> () {
    println!("hi")
}

/*
use std::env;
use std::io::Error;
use std::path::Path;
use tokio;
use tokio::net::{TcpStream, UnixStream};
use tokio::prelude::*;
use tokio_codec::Framed;
use std::net::SocketAddr;
use std::str::FromStr;
use tokio_scgi::{SCGICodec, SCGIRequest};

fn main() -> Result<(), Error> {
    let endpoint = env::args().nth(1).unwrap_or("/tmp/scgi.sock".to_string());

    let mut headers = Vec::new();
    headers.push(("USERNAME".to_string(), "bort".to_string()));

    let mut sendme = Vec::new();
    sendme.push(SCGIRequest::Headers(headers));
    sendme.push(SCGIRequest::BodyFragment("my name is also bort".to_string().into_bytes()));

    if endpoint.contains('/') {
        // Probably a path to a file
        let path = Path::new(&endpoint);
        println!("Connecting to Unix {}", path.display());
        send_request(UnixStream::connect(&path), sendme);
        Ok(())
    } else {
        // Probably a TCP endpoint
        let addr = SocketAddr::from_str(endpoint.as_str())
            .expect(format!("Invalid endpoint: {}", endpoint).as_str());
        println!("Connecting to TCP {}", addr);
        send_request(TcpStream::connect(&addr), sendme);
        Ok(())
    }
}

fn send_request<T: Future + Stream>(foo: T, sendme: Vec<SCGIRequest>) {
    foo.and_then(|stream| {
        Framed::new(stream, SCGICodec::new()).send(sendme.get(0).expect("a").clone())
        //.send_all(stream::iter_ok(sendme))
    }).and_then(|result| {
        println!("Send result: {:?}", result);
        Ok(())
    }).map_err(|err| {
        println!("Something failed: {:?}", err);
    }).poll().expect("c");
}
*/