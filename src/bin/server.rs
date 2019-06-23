#![deny(warnings, rust_2018_idioms)]

use std::env;
use std::fs;
use std::io::{Error, ErrorKind};
use std::path::Path;
use tokio;
use tokio::net::UnixListener;
use tokio::prelude::*;
use tokio_codec::Framed;

fn try_delete(path: &Path) -> Result<(), Error> {
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
        })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Allow passing an address to listen on as the first argument of this
    // program, but otherwise we'll just set up our TCP listener on
    // 127.0.0.1:8080 for connections.
    let sock_path_str = env::args().nth(1).unwrap_or("/tmp/scgi.sock".to_string());
    let sock_path = Path::new(&sock_path_str);

    // Try to delete the socket file. Avoids AddrInUse errors.
    try_delete(&sock_path).unwrap();

    // Next up we create a TCP listener which will listen for incoming
    // connections. This TCP listener is bound to the address we determined
    // above and must be associated with an event loop, so we pass in a handle
    // to our event loop. After the socket's created we inform that we're ready
    // to go and start accepting connections.
    let socket = UnixListener::bind(&sock_path)?;
    println!("Listening on {}", sock_path.display());

    // Mark file rw-all so that httpd can write to it
    let mut sock_perms = fs::metadata(&sock_path).unwrap().permissions();
    sock_perms.set_readonly(false);
    fs::set_permissions(&sock_path, sock_perms).unwrap();

    // Here we convert the `TcpListener` to a stream of incoming connections
    // with the `incoming` method. We then define how to process each element in
    // the stream with the `for_each` method.
    //
    // This combinator, defined on the `Stream` trait, will allow us to define a
    // computation to happen for all items on the stream (in this case TCP
    // connections made to the server).  The return value of the `for_each`
    // method is itself a future representing processing the entire stream of
    // connections, and ends up being our server.
    let done = socket
        .incoming()
        .map_err(|e| println!("failed to accept socket; error = {:?}", e))
        .for_each(move |socket| {
            // Once we're inside this closure this represents an accepted client
            // from our server. The `socket` is the client connection (similar to
            // how the standard library operates).
            //
            // We're parsing each socket with the `BytesCodec` included in `tokio_io`,
            // and then we `split` each codec into the reader/writer halves.
            //
            // See https://docs.rs/tokio-codec/0.1/src/tokio_codec/bytes_codec.rs.html
            let (_writer, reader) = Framed::new(socket, tokio_scgi::SCGICodec::new()).split();

            let processor = reader
                .for_each(|bytes| {
                    println!("bytes: {:?}", bytes);
                    Ok(())
                })
                // After our copy operation is complete we just print out some helpful
                // information.
                .and_then(|()| {
                    println!("Socket received FIN packet and closed connection");
                    Ok(())
                })
                .or_else(|err| {
                    println!("Socket closed with error: {:?}", err);
                    // We have to return the error to catch it in the next ``.then` call
                    Err(err)
                })
                .then(|result| {
                    println!("Socket closed with result: {:?}", result);
                    Ok(())
                });

            // And this is where much of the magic of this server happens. We
            // crucially want all clients to make progress concurrently, rather than
            // blocking one on completion of another. To achieve this we use the
            // `tokio::spawn` function to execute the work in the background.
            //
            // This function will transfer ownership of the future (`msg` in this
            // case) to the Tokio runtime thread pool that. The thread pool will
            // drive the future to completion.
            //
            // Essentially here we're executing a new task to run concurrently,
            // which will allow all of our clients to be processed concurrently.
            tokio::spawn(processor)
        });

    // And finally now that we've define what our server is, we run it!
    //
    // This starts the Tokio runtime, spawns the server task, and blocks the
    // current thread until all tasks complete execution. Since the `done` task
    // never completes (it just keeps accepting sockets), `tokio::run` blocks
    // forever (until ctrl-c is pressed).
    tokio::run(done);
    Ok(())
}
