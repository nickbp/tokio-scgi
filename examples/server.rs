#![deny(warnings)]

use bytes::{BufMut, BytesMut};
use futures::{SinkExt, StreamExt};
use std::env;
use std::fs;
use std::io::{Error, ErrorKind};
use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::SystemTime;
use tokio;
use tokio::net::{TcpListener, UnixListener};
use tokio::prelude::*;
use tokio::task;
use tokio_scgi::server::{SCGICodec, SCGIRequest};
use tokio_util::codec::Framed;

fn syntax() -> Error {
    println!(
        "Syntax: {} </path/to/unix.sock or tcp-host:1234>",
        env::args().nth(0).unwrap()
    );
    Error::new(ErrorKind::InvalidInput, "Missing required argument")
}

#[tokio::main]
async fn main() -> Result<(), std::io::Error> {
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
        let bind = unix_init(endpoint)?;
        loop {
            let (conn, _addr) = bind.accept().await?;
            task::spawn(async move {
                match serve(conn).await {
                    Err(e) => {
                        println!("Error serving UDS session: {:?}", e);
                    }
                    Ok(()) => {
                        println!("Served UDS request");
                    }
                };
            });
        }
    } else {
        // Probably a TCP endpoint, try to resolve it in case it's a hostname
        let bind = tcp_init(endpoint).await?;
        loop {
            let (conn, addr) = bind.accept().await?;
            task::spawn(async move {
                match serve(conn).await {
                    Err(e) => {
                        println!("Error when serving TCP session from {:?}: {:?}", addr, e);
                    }
                    Ok(()) => {
                        println!("Served TCP request from {:?}", addr);
                    }
                };
            });
        }
    }
}

fn unix_init(path_str: String) -> Result<UnixListener, Error> {
    let path = Path::new(&path_str);
    // Try to delete the socket file. Avoids AddrInUse errors. No-op if already missing.
    fs::remove_file(path)
        .and_then(|()| {
            println!("Deleted existing {}", path.display());
            Ok(())
        })
        .or_else(|err| {
            // Ignore no-op case of deleting file that already doesn't exist
            match err.kind() {
                ErrorKind::NotFound => Ok(()),
                _ => {
                    println!("Failed to delete {}: {}", path.display(), err);
                    Err(err)
                }
            }
        })?;

    let socket = UnixListener::bind(&path)?;
    println!("Listening on {}", path.display());

    // Mark file rw-all so that clients can write to it
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();

    Ok(socket)
}

async fn tcp_init(endpoint_str: String) -> Result<TcpListener, Error> {
    let addr = endpoint_str
        .to_socket_addrs()
        .expect(format!("Invalid TCP endpoint '{}'", endpoint_str).as_str())
        .next()
        .unwrap();

    let socket = TcpListener::bind(&addr).await?;
    println!("Listening on {}", addr);

    Ok(socket)
}

macro_rules! http_response {
    ($response_code:expr, $content_type:expr, $content:expr) => {
        format!(
            "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n{}",
            ($response_code),
            ($content_type),
            ($content).len(),
            ($content)
        )
        .into_bytes()
    };
}

async fn serve<C>(conn: C) -> Result<(), Error>
where
    C: AsyncRead + AsyncWrite + std::marker::Send + std::marker::Unpin + std::fmt::Debug,
{
    let mut handler = SampleHandler::new();
    let mut framed = Framed::new(conn, SCGICodec::new());

    loop {
        match framed.next().await {
            None => {
                // SCGI request not ready: loop for more rx data
                println!("Request read returned None, resuming read");
            }
            Some(Err(e)) => {
                // RX error: return error and abort
                return Err(Error::new(
                    ErrorKind::Other,
                    format!("Error when waiting for request: {}", e),
                ));
            }
            Some(Ok(request)) => {
                // Got SCGI request: pass to handler
                match handler.handle(request) {
                    Ok(Some(r)) => {
                        // Response ready: send and exit
                        return framed.send(r).await;
                    }
                    Ok(None) => {
                        // Response not ready: loop for more rx data
                        println!("Request data is incomplete, resuming read");
                    }
                    Err(e) => {
                        // Handler error: respond with formatted error message
                        return framed.send(handle_error(e)).await;
                    }
                }
            }
        }
    }
}

struct SampleHandler {
    /// Copy of headers, only used if the request is streamed.
    headers: Vec<(String, String)>,

    /// Accumulated body received so far.
    body: BytesMut,

    /// The amount of unconsumed body remaining, according to Content-Length.
    body_remaining: usize,
}

/// A sample implementation of an SCGI service that uses `SCGICodec` for parsing inbound requests,
/// and sending back HTML responses based on those requests.
impl SampleHandler {
    /// Returns a `SampleHandler` for exercising handling an HTTP request and sending back a sample
    /// HTML response.
    pub fn new() -> SampleHandler {
        SampleHandler {
            headers: Vec::new(),
            body: BytesMut::new(),
            body_remaining: 0,
        }
    }

    /// This is where you'd put in your code accepting the request and returning a response.
    fn handle(&mut self, req: SCGIRequest) -> Result<Option<Vec<u8>>, Error> {
        match req {
            // Accept the header and any POSTed payload in the body.
            SCGIRequest::Request(headers, body) => {
                ////////////////////
                // Note for implementors: You can build your own logic for deciding when the request
                // data has all arrived, based on the payload format you're dealing with. This is
                // just a sample. For example, instead of looking at Content-Length you might
                // instead look for some signal in the data itself that it's reached the end of the
                // payload. However, in most cases you shouldn't need to do any of this unless you
                // specifically want to support e.g. streaming content in your service. Basic
                // requests should in practice be fully encapsulated by the initial
                // SCGIRequest::Request.
                ////////////////////

                // Check whether we should tell upstream to wait for more content, based on whether
                // the body we've gotten so far matches the value of Content-Length. If it's missing
                // then we just give up and assume we have everything. Per SCGI spec, Content-Length
                // should always be the first header. But let's play it safe and (potentially) check
                // all of the headers.
                for pair in headers.iter() {
                    if !pair.0.eq("Content-Length") {
                        continue;
                    }

                    // If the Content-Length value doesn't parse then return an error.
                    match pair.1.parse() {
                        Ok(content_length) => {
                            if body.len() >= content_length {
                                // Looks like we've gotten everything. Send the response and exit.
                                // (The is the common case)
                                return Ok(Some(build_response(&headers, &body)));
                            } else {
                                // Save current content, send empty/no-op response while we wait for
                                // the remainder.
                                self.headers = headers;
                                self.body_remaining = content_length - body.len();
                                self.body = body;
                                return Ok(None);
                            }
                        }
                        Err(e) => {
                            return Err(Error::new(
                                // Use the same ErrorKind used by the parser so that we return HTTP 400
                                ErrorKind::InvalidData,
                                format!("Content-Length '{}' is not an integer: {}", pair.1, e),
                            ));
                        }
                    }
                }
                // No Content-Length was found. Assume we've got everything and avoid more reads.
                Ok(Some(build_response(&headers, &body)))
            }
            // Handle additional body fragments. This should only happen if we had returned Continue
            // above. For basic requests, this additional handling shouldn't be necessary. See above
            // "Note for implementors"
            SCGIRequest::BodyFragment(more_body) => {
                if self.body_remaining <= more_body.len() {
                    self.body_remaining = 0;
                } else {
                    self.body_remaining -= more_body.len()
                }
                self.body.reserve(more_body.len());
                self.body.put(more_body);

                if self.body_remaining <= 0 {
                    // We've gotten all the remaining data. Send the response and exit.
                    Ok(Some(build_response(&self.headers, &self.body)))
                } else {
                    // More data remains, continue waiting for it and return (another) empty noop.
                    Ok(None)
                }
            }
        }
    }
}

fn handle_error(e: Error) -> Vec<u8> {
    let msg = format!("{}", e);
    match e.kind() {
        // InvalidData implies an error from the SCGI codec.
        // Let's assume the request was malformed.
        ErrorKind::InvalidData => http_response!("400 Bad Request", "text/plain", msg),
        // Handler should have just produced an error response for e.g. HTTP 404.
        // Therefore assume any other Err cases are due to a handler bug.
        _ => {
            println!("Replying with HTTP 500 due to handler error: {}", e);
            http_response!("500 Internal Server Error", "text/plain", msg)
        }
    }
}

fn build_response(headers: &Vec<(String, String)>, body: &BytesMut) -> Vec<u8> {
    let epoch_secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let body_str = match String::from_utf8(body.to_vec()) {
        // Printable content with minimal effort at avoiding HTML injection:
        Ok(s) => format!("{}", s.replace('<', "&lt;").replace('>', "&gt;")),
        // Not printable content, fall back to printing as list of dec codes:
        Err(_e) => format!("{:?}", body.to_vec()),
    };
    let content = format!(
        "<html><head><title>scgi-sample-server</title></head><body>
<p>hello! the epoch time is {:?}, and your request was:</p>
<ul><li>headers: {:?}</li>
<li>body ({} bytes): {}</li></ul>
</body></html>\n",
        epoch_secs,
        headers,
        body.len(),
        body_str
    );
    http_response!("200 OK", "text/html", content)
}
