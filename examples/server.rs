#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use std::env;
use std::fs;
use std::io::{Error, ErrorKind};
use std::net::ToSocketAddrs;
use std::path::Path;
use std::time::SystemTime;
use tokio;
use tokio::net::{TcpListener, UnixListener};
use tokio::prelude::*;
use tokio_codec::Framed;
use tokio_scgi::abortable_stream::{AbortableItem, AbortableStream};
use tokio_scgi::server::{SCGICodec, SCGIRequest};

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

    if endpoint.contains('/') {
        // Probably a path to a file, assume the argument is a unix socket
        tokio::run(
            unix_init(endpoint)?
                .incoming()
                .map_err(|e| println!("Unix socket failed: {:?}", e))
                .for_each(|conn| serve(conn)),
        );
    } else {
        // Probably a TCP endpoint, try to resolve it in case it's a hostname
        tokio::run(
            tcp_init(endpoint)?
                .incoming()
                .map_err(|e| println!("TCP socket failed: {:?}", e))
                .for_each(|conn| serve(conn)),
        );
    }
    Ok(())
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

fn tcp_init(endpoint_str: String) -> Result<TcpListener, Error> {
    let addr = endpoint_str
        .to_socket_addrs()
        .expect(format!("Invalid TCP endpoint '{}'", endpoint_str).as_str())
        .next()
        .unwrap();

    let socket = TcpListener::bind(&addr)?;
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

fn serve<T>(conn: T) -> tokio::executor::Spawn
where
    T: AsyncRead + AsyncWrite + 'static + std::marker::Send + std::fmt::Debug,
{
    let mut handler = SampleHandler::new();
    let (tx_scgi, rx_scgi) = Framed::new(conn, SCGICodec::new()).split();
    // Request flow:
    // 1. rx_scgi is queried for request data. It blocks until data is available.
    // 2. The raw request data is received and passed to SCGICodec. which consumes it and returns
    //    an SCGI Request or BodyFragment when enough of the raw data has arrived
    // 3. SCGICodec consumes the raw request data, and waits for at least the complete SCGI headers.
    //    At this point SCGICodec will return a Request, followed by zero or more BodyFragments as
    //    any more raw request data comes in.
    // 4. The Request and any BodyFragments are passed to sample handler, which then returns a
    //    response.
    // 5. Sample handler returns Continue or Stop with its response data, which can be an empty vec.
    // 6. In both Continue and Stop cases, the returned response data is sent back to the client
    //    as-is using tx_scgi. In this direction the SCGICodec functions as a passthrough.
    // 7a. If Stop was returned, a bit is set to ensure that the stream returns None the next time
    //     it's polled. In particular it will avoid reading from rx_scgi again, since sample handler
    //     has effectively said there's nothing left to be read from there.
    // 7b. If Continue was returned, rx_scgi is queried for more data and the cycle continues.
    let session = tx_scgi
        .send_all(AbortableStream::with_err_conv(
            rx_scgi.and_then(move |request| match handler.handle(request) {
                Ok(r) => Ok(r),
                Err(e) => Ok(AbortableItem::Stop(handle_error(e))),
            }),
            // We don't see errors produced by the SCGICodec itself, so we give AbortableStream this
            // custom error handler to turn any parsing errors into nice HTML responses:
            |err| Some(handle_error(err)),
        ))
        .then(|send_all_result| {
            match send_all_result {
                Ok(_session) => {
                    // Session ended successfully
                    Ok(())
                }
                Err(e) => {
                    println!("Unhandled session error: {:?}", e);
                    // Keep spawn() typing happy:
                    Err(())
                }
            }
        });
    tokio::spawn(session)
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
    fn handle(&mut self, req: SCGIRequest) -> Result<AbortableItem<Vec<u8>>, Error> {
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
                                // Looks like we've gotten everything. Return the response now.
                                // (The is the common case)
                                return Ok(AbortableItem::Stop(build_response(&headers, &body)));
                            } else {
                                // Save current content, send empty/no-op response while we wait for
                                // the remainder.
                                self.headers = headers;
                                self.body_remaining = content_length - body.len();
                                self.body = body;
                                return Ok(AbortableItem::Continue(Vec::new()));
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
                Ok(AbortableItem::Stop(build_response(&headers, &body)))
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
                    // We've gotten all the remaining data. Send the response, and tell upstream to
                    // not read from the socket again.
                    Ok(AbortableItem::Stop(build_response(
                        &self.headers,
                        &self.body,
                    )))
                } else {
                    // More data remains, continue waiting for it and return (another) empty noop.
                    Ok(AbortableItem::Continue(Vec::new()))
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
        Err(_e) => format!("{:?}", body.to_vec())
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
