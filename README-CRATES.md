# tokio_scgi

This is a Rust library which implements support for building [SCGI](https://python.ca/scgi/) servers and clients. It comes in the form of a Tokio Codec which can be used in asynchronous code, but it can also be invoked directly in synchronous or non-Tokio code.

SCGI is a [simple and efficient](http://python.ca/scgi/protocol.txt) protocol for communicating between frontend web servers and backend applications over a TCP or local Unix socket. It compares (favorably) to [FastCGI](https://en.wikipedia.org/wiki/FastCGI), another protocol with a similar purpose. This library provides support for writing both SCGI servers and clients in Rust, with support for both TCP and Unix sockets.

For more information including documentation and examples, see [https://github.com/nickbp/tokio-scgi](the project repo).
