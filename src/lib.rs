#![deny(warnings, rust_2018_idioms)]

//! SCGI request codec for Tokio.
//!
//! This crate provides codecs for creating and parsing SCGI requests, for use by web servers to query SCGI services and backend services to serve SCGI endpoints.
//! Working examples are provided for asynchronous SCGI servers and clients. Tests meanwhile provide examples of invoking the codecs directly.

/// For an SCGI server (usually a backend service): Parses SCGI requests and sends back raw byte responses.
pub mod server;

/// For an SCGI client (usually a web server): Builds SCGI requests and receives raw byte responses.
pub mod client;
