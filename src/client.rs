#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use std::io;
use tokio_codec::{Decoder, Encoder};

const NUL: u8 = b'\0';

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SCGIRequest {
    Headers(Vec<(String, String)>),
    BodyFragment(Vec<u8>),
}

/// A `Codec` implementation that creates and parses SCGI requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SCGICodec { }

impl SCGICodec {
    /// Returns a `SCGIClientCodec` for creating SCGI-format requests.
    pub fn new() -> SCGICodec {
        SCGICodec { }
    }
}

impl Decoder for SCGICodec {
    type Item = Vec<u8>;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Vec<u8>>, io::Error> {
        // Forward content (HTTP response, typically?) as-is
        // TODO consider using an existing HTTP library to accept an HTTP response object, but also
        // allow raw passthrough in the response as well.
        Ok(Some(buf.split_to(buf.len()).to_vec()))
    }
}

/// Creates and produces SCGI requests. Invoke once with `Headers`, followed by zero or more calls
/// with `BodyFragment`.
impl Encoder for SCGICodec {
    type Item = SCGIRequest;
    type Error = io::Error;

    fn encode(&mut self, data: SCGIRequest, buf: &mut BytesMut) -> Result<(), io::Error> {
        match data {
            SCGIRequest::Headers(env_map) => {
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
                    netstring_size_str.len() + 1/*:*/ + sum_header_size + 1, /*,*/
                );

                // Insert the header content into the reserved buffer.
                buf.put(netstring_size_str);
                buf.put(b':');
                for (k, v) in &env_map {
                    buf.put(k);
                    buf.put(NUL);
                    buf.put(v);
                    buf.put(NUL);
                }
                buf.put(b',');
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
