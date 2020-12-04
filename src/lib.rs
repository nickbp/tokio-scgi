#![deny(warnings)]

//! SCGI request codec for Tokio.
//!
//! This crate provides codecs for creating and parsing SCGI requests. Web servers can use this to query SCGI services as clients. Backend services can use this to serve SCGI endpoints to web servers. For example, you can build a backend service in Rust that serves responses over SCGI to a frontend NGINX server. Check the NGINX documentation for info on how to configure SCGI.
//! Working examples of Tokio-based SCGI servers and clients are provided in the project examples. Tests meanwhile provide examples of invoking the codecs directly.

/// Codec for SCGI servers, such as backend services: Parses SCGI requests and sends back raw byte responses to forward back to querying clients.
pub mod server;

/// Codec for SCGI clients, such as web servers: Builds SCGI requests and receives raw byte responses to forward back to querying clients.
pub mod client;
