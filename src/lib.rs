#![deny(warnings, rust_2018_idioms)]

/// For an SCGI server (: Parses SCGI requests and sends back raw byte responses.
pub mod server;

/// For an SCGI client (usually a web server): Builds SCGI requests and receives raw byte responses.
pub mod client;
