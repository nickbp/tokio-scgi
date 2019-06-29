#![deny(warnings, rust_2018_idioms)]

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
        Err(syntax())
    } else if endpoint.contains('/') {
        // Probably a path to a file, assume the argument is a unix socket
        tokio::run(
            unix_init(endpoint)?
                .incoming()
                .map_err(|e| println!("Unix socket failed: {:?}", e))
                .for_each(|conn| serve(conn)),
        );
        Ok(())
    } else {
        // Probably a TCP endpoint, try to resolve it in case it's a hostname
        let addr = endpoint
            .to_socket_addrs()
            .expect(format!("Invalid TCP endpoint '{}'", endpoint).as_str())
            .next()
            .unwrap();
        println!("Listening on {}", addr);
        tokio::run(
            TcpListener::bind(&addr)?
                .incoming()
                .map_err(|e| println!("TCP socket failed: {:?}", e))
                .for_each(|conn| serve(conn)),
        );
        Ok(())
    }
}

fn unix_init(path_str: String) -> Result<UnixListener, Error> {
    // Try to delete the socket file. Avoids AddrInUse errors. No-op if already missing.
    let path = Path::new(&path_str);
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
    let (tx_scgi, rx_scgi) = Framed::new(conn, SCGICodec::new()).split();
    let session = tx_scgi
        .send_all(
            // TODO i think what's happening is we're trying to query for more data here
            // ideally we'd have handler() return a different code that says 'here's my response but
            // stop now' and then this code would cut the stream without trying another read.
            rx_scgi
                .and_then(|request| future::lazy(move || handler(request)))
                .then(|rx_result| {
                    match &rx_result {
                        Ok(_) => {
                            println!("Great job team");
                            rx_result
                        }
                        Err(e) => {
                            println!("handler returned error: {}", e);
                            let msg = format!("{}", e);
                            match e.kind() {
                                ErrorKind::InvalidData => {
                                    // InvalidData implies an error from the SCGI codec.
                                    // Let's assume the request was malformed.
                                    Ok(http_response!("400 Bad Request", "text/plain", msg))
                                }
                                ErrorKind::NotFound => {
                                    // Example mapping of other handler errors.
                                    Ok(http_response!("404 Not Found", "text/plain", msg))
                                }
                                _ => Ok(http_response!(
                                    "500 Internal Server Error",
                                    "text/plain",
                                    msg
                                )),
                            }
                        }
                    }
                }),
        )
        .then(|send_all_result| {
            match send_all_result {
                Ok(_session) => {
                    println!("Session ended successfully somehow");
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

/// This is where you'd put in your code accepting the request and returning a response.
fn handler(req: SCGIRequest) -> Result<Vec<u8>, Error> {
    match req {
        // Accept the header and any POSTed payload in the body.
        SCGIRequest::Request(headers, body) => {
            let epoch_secs = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap();
            // This demo does not protect against HTML injection.
            let content = format!(
                "<html><head><title>scgi-demo-server</title></head><body>
<p>hello! the epoch time is {:?}, and your request was:</p>
<ul><li>headers: {:?}</li>
<li>body: {:?}</li></ul>
</body></html>\n",
                epoch_secs, headers, body
            );
            Ok(http_response!("200 OK", "text/html", content))
        }

        // Support for streaming/multipart content depends on the streaming format used. Your
        // handling might accumulate (or flush to disk) the data as it arrives in BodyFragments.
        // Your implementation would then also need to use a strategy like one of the following to
        // know when no more BodyFragments are going to arrive:
        // - Look at the "Content-Length" HTTP header and use that to decide when the data has all
        //   arrived.
        // - Look for some signal in the data itself saying when to stop expecting new data.
        // However, in practice none of this should be needed unless you specifically want to
        // support streaming content in your service. Basic requests should be fully encapsulated by
        // the initial SCGIRequest::Request.
        SCGIRequest::BodyFragment(_more_body) => {
            // This implementation closes the connection after the first Request, so this shouldn't
            // be reachable anyway.
            Err(Error::new(
                ErrorKind::InvalidData,
                "Multiple body fragments are not supported",
            ))
        }
    }
}
