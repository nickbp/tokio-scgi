#![deny(warnings)]

use bytes::{BufMut, BytesMut};
use std::io;
use tokio_util::codec::{Decoder, Encoder};

const NUL: u8 = b'\0';

/// A parsed SCGI request header with key/value header data, and/or bytes from the raw request body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SCGIRequest {
    /// The Vec contains the headers. The BytesMut optionally contains raw byte data from
    /// the request body, which may be followed by additional `BodyFragment`s in later calls.
    /// The `Content-Length` header, required by SCGI, can be used to detect whether to wait for
    /// additional `BodyFragment`s.
    Request(Vec<(String, String)>, BytesMut),

    /// Additional body fragment(s), used for streaming fragmented request body data. These should
    /// only be relevant in cases where the leading `Request` value doesn't contain all of the body.
    BodyFragment(BytesMut),
}

/// A `Codec` implementation that creates SCGI requests for SCGI clients like web servers.
/// The Encoder accepts `SCGIRequest` objects containing header/body request data and encodes them for
/// sending to an SCGI server. The Decoder passes through the raw response returned by the SCGI server.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SCGICodec {}

impl SCGICodec {
    /// Returns a client `SCGICodec` for creating SCGI-format requests for use by SCGI clients
    /// like web servers.
    pub fn new() -> SCGICodec {
        SCGICodec {}
    }
}

/// Passes through any response data as-is. To be handled by the requesting client.
impl Decoder for SCGICodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
        // Forward content (HTTP response, typically?) as-is
        Ok(Some(buf.split_to(buf.len())))
    }
}

/// Creates and produces SCGI requests. Invoke once with `Request`, followed by zero or more calls
/// with `BodyFragment`.
impl Encoder<SCGIRequest> for SCGICodec {
    type Error = io::Error;

    fn encode(&mut self, data: SCGIRequest, buf: &mut BytesMut) -> Result<(), io::Error> {
        match data {
            SCGIRequest::Request(env_map, body) => {
                // Calculate size needed for header netstring
                let mut sum_header_size: usize = 0;
                for (k, v) in &env_map {
                    // While we're iterating over the keys/values, do some basic validation per the
                    // SCGI protocol spec.
                    if k.len() == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Keys in request header cannot be empty"),
                        ));
                    }
                    if k.as_bytes().contains(&NUL) || v.as_bytes().contains(&NUL) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("Keys/values in request header cannot contain NUL character"),
                        ));
                    }
                    // Include 2 x NUL in size:
                    sum_header_size += k.len() + 1/*NUL*/ + v.len() + 1/*NUL*/;
                }
                let netstring_size_str = sum_header_size.to_string();
                // Include ':' and ',' in buffer, not included in netstring size:
                buf.reserve(
                    netstring_size_str.len() + 1/*:*/ + sum_header_size + 1/*,*/ + body.len(),
                );

                // Insert the header content into the reserved buffer.
                buf.put_slice(netstring_size_str.as_bytes());
                buf.put_u8(b':');
                for (k, v) in &env_map {
                    buf.put(k.as_bytes());
                    buf.put_u8(NUL);
                    buf.put(v.as_bytes());
                    buf.put_u8(NUL);
                }
                buf.put_u8(b',');

                // Add any body content after the header
                buf.put(body);
            }
            SCGIRequest::BodyFragment(fragment) => {
                // Forward content as-is
                buf.reserve(fragment.len());
                buf.put(fragment);
            }
        }
        Ok(())
    }
}
