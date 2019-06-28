#![deny(warnings, rust_2018_idioms)]

use std::env;
use std::fs;
use std::io::{Error, ErrorKind};
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use tokio;
use tokio::net::{TcpListener, UnixListener};
use tokio::prelude::*;
use tokio_codec::Framed;
use tokio_scgi::server::{SCGICodec, SCGIRequest};

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

fn main() -> Result<(), Error> {
    let endpoint = env::args().nth(1).unwrap_or("/tmp/scgi.sock".to_string());
    if endpoint.contains('/') {
        // Probably a path to a file
        tokio::run(
            unix_init(endpoint)?
                .incoming()
                .map_err(|e| println!("Socket failed: {:?}", e))
                .for_each(|socket| serve(socket)),
        );
        Ok(())
    } else {
        // Probably a TCP endpoint
        let addr = SocketAddr::from_str(endpoint.as_str())
            .expect(format!("Invalid endpoint: {}", endpoint).as_str());
        println!("Listening on {}", addr);
        tokio::run(
            TcpListener::bind(&addr)?
                .incoming()
                .map_err(|e| println!("Socket failed: {:?}", e))
                .for_each(|socket| serve(socket)),
        );
        Ok(())
    }
}

fn serve<T: AsyncRead + AsyncWrite + 'static>(socket: T) -> tokio::executor::Spawn
where
    T: std::marker::Send,
{
    let (tx_scgi, rx_scgi) = Framed::new(socket, SCGICodec::new()).split();
    let session = tx_scgi.send_all(rx_scgi.and_then(respond)).then(|res| {
        if let Err(e) = res {
            println!("failed to process connection; error = {:?}", e);
        }
        Ok(())
    });
    tokio::spawn(session)
}

fn respond(req: SCGIRequest) -> Box<dyn Future<Item = Vec<u8>, Error = Error> + Send> {
    let f = future::lazy(move || {
        let content = format!(
            "<html><head><title>scgi-demo</title></head><body>\
             <p>hello! request was:</p>\
             <p>{:?}</p>
             </body></html>\n",
            req
        );
        Ok(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            content.len(),
            content
        )
        .into_bytes())
    });

    Box::new(f)
}
