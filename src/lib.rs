#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use std::{io, mem};
use tokio_codec::{Decoder, Encoder};

const NUL: u8 = b'\0';
/// The maximum size in bytes of a single header name or value. This limit is far greater than the
/// 4k-8k that is enforced by most web servers.
const MAX_HEADER_STRING_BYTES: usize = 32 * 1024;
/// The maximum size in bytes for all header content. This limit is far greater than the 4k-8k that
/// is enforced by most web servers.
const MAX_HEADER_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
enum CodecState {
    /// Getting the initial netstring size.
    /// => HeaderKey when ':' is encountered and header_size > 0.
    /// => ContentSeparator when ':' is encountered and header_size == 0.
    HeaderSize,

    /// Getting a header key.
    /// => HeaderValue when NUL is encountered.
    HeaderKey,

    /// Getting a header value.
    /// => HeaderKey when NUL is encountered and remaining_header_size > 0.
    /// => ContentSeparator when NUL is encountered and remaining_header_size == 0.
    HeaderValue,

    /// Getting the ',' separating headers from content.
    /// => Content when ',' is encountered.
    ContentSeparator,

    /// Forwarding any payload content, may match CONTENT_SIZE header.
    Content,
}

/// A `Codec` implementation that creates and parses SCGI requests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SCGICodec {
    // DECODER
    /// Decoder state. See `CodecState` for transition info.
    decoder_state: CodecState,

    /// The amount of unconsumed header remaining. There should be a ',' at this index.
    header_remaining: usize,

    /// The accumulated header_key, assigned when exiting HeaderKey state and cleared/consumed when
    /// leaving HeaderValue state
    header_key: String,

    /// The accumulated headers, populated when leaving HeaderValue states and forwarded to caller
    /// when entering Content state from last HeaderValue state. Intentionally using a `Vec` to
    /// preserve ordering.
    headers: Vec<(String, String)>,

    /// Pointer to index where searches should begin for a character in the provided buffer. Must be
    /// reset to 0 after consuming from the buffer.
    next_search_index: usize,

    // ENCODER
    /// Encoder state. See `CodecState` for transition info.
    encoder_state: CodecState,
}

impl SCGICodec {
    /// Returns a `SCGICodec` for creating and/or parsing SCGI-format requests.
    pub fn new() -> SCGICodec {
        SCGICodec {
            decoder_state: CodecState::HeaderSize,
            header_remaining: 0,
            header_key: String::new(),
            headers: Vec::new(),
            next_search_index: 0,
            encoder_state: CodecState::HeaderSize,
        }
    }

    /// Loops and consumes all available headers in the buffer, returning a `SCGIRequest::Headers`
    /// result if complete headers were available, or `None` if the end of the headers wasn't yet
    /// reachable in the buffer.
    fn consume_headers(&mut self, buf: &mut BytesMut) -> Result<Option<SCGIRequest>, io::Error> {
        loop {
            match self.decoder_state {
                CodecState::ContentSeparator => {
                    // Just consume the ',' that should be present, or complain if it isn't found
                    if buf.len() == 0 {
                        return Ok(None);
                    } else if buf[0] == b',' {
                        // Cut the ',' from the buffer, return headers and switch to content mode
                        buf.split_to(1);
                        self.next_search_index = 0;
                        self.decoder_state = CodecState::Content;
                        return Ok(Some(SCGIRequest::Headers(mem::replace(
                            &mut self.headers,
                            Vec::new(),
                        ))));
                    } else {
                        // Should always have the comma, missing it implies corrupt input.
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Missing ',' separating headers from content",
                        ));
                    }
                }
                CodecState::HeaderKey | CodecState::HeaderValue => {
                    if let Some(end_offset) =
                        buf[self.next_search_index..].iter().position(|b| *b == NUL)
                    {
                        // Consume string and trailing NUL from buffer:
                        let bytes_with_nul = buf.split_to(self.next_search_index + end_offset + 1);
                        self.next_search_index = 0;
                        self.header_remaining -= bytes_with_nul.len();
                        // Found NUL for end of a header string, consume
                        match self.decoder_state {
                            CodecState::HeaderKey => {
                                // Store the header key and enter header value state.
                                self.header_key = consume_header_string(bytes_with_nul)?;
                                self.decoder_state = CodecState::HeaderValue;
                            }
                            CodecState::HeaderValue => {
                                // Store the header key+value entry and enter header key OR content state.
                                self.headers.push((
                                    mem::replace(&mut self.header_key, String::new()),
                                    consume_header_string(bytes_with_nul)?,
                                ));
                                if self.header_remaining > 0 {
                                    // Still in headers, set up search for next key
                                    self.decoder_state = CodecState::HeaderKey;
                                } else {
                                    // Reached end of headers, but consume separator ',' before returning
                                    self.decoder_state = CodecState::ContentSeparator;
                                }
                            }
                            _ => panic!("Unexpected state {:?}", self.decoder_state),
                        }
                    } else {
                        // No NUL available yet, try again
                        self.next_search_index = buf.len();
                        if self.next_search_index > MAX_HEADER_STRING_BYTES {
                            // This string is getting to be way too long. Bad data? Give up.
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "Header key or value size exceeds maximum {} bytes",
                                    MAX_HEADER_STRING_BYTES
                                )
                                .as_str(),
                            ));
                        }
                        return Ok(None);
                    }
                }
                CodecState::HeaderSize | CodecState::Content => {
                    panic!("Unexpected state {:?}", self.decoder_state);
                }
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SCGIRequest {
    Headers(Vec<(String, String)>),
    BodyFragment(Vec<u8>),
}

impl Decoder for SCGICodec {
    type Item = SCGIRequest;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<SCGIRequest>, io::Error> {
        match self.decoder_state {
            CodecState::HeaderSize => {
                // Search for ':' which follows the header size int
                if let Some(end_offset) = buf[self.next_search_index..]
                    .iter()
                    .position(|b| *b == b':')
                {
                    // Consume size string and trailing ':' from start of buffer
                    // Store the header size and enter header key state
                    self.header_remaining =
                        consume_header_size(buf.split_to(self.next_search_index + end_offset + 1))?;
                    if self.header_remaining > MAX_HEADER_BYTES {
                        // This declared size is way too long. Bad data? Give up. We just want to
                        // avoid accumulating too much data on the header `Vec`. When we've consumed
                        // all `header_remaining` bytes we will switch to content forwarding mode.
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Header size exceeds maximum {} bytes", MAX_HEADER_BYTES)
                                .as_str(),
                        ));
                    }
                    self.next_search_index = 0;
                    if self.header_remaining > 0 {
                        // Start consuming header(s)
                        self.decoder_state = CodecState::HeaderKey;
                        self.consume_headers(buf)
                    } else {
                        // No headers, skip straight to content separator.
                        // According to the scgi spec this shouldn't happen but let's allow it.
                        self.decoder_state = CodecState::ContentSeparator;
                        // Handles consuming the content separator (and emitting the empty headers)
                        // internally.
                        self.consume_headers(buf)
                    }
                } else {
                    // No ':' yet, try again
                    self.next_search_index = buf.len();
                    Ok(None)
                }
            }
            CodecState::HeaderKey | CodecState::HeaderValue | CodecState::ContentSeparator => {
                // Resumable internal loop to consume all available headers in buffer
                self.consume_headers(buf)
            }
            CodecState::Content => {
                // Consume and forward whatever was received
                Ok(Some(SCGIRequest::BodyFragment(
                    buf.split_to(buf.len()).to_vec(),
                )))
            }
        }
    }
}

fn consume_header_size(bytes_with_colon: BytesMut) -> Result<usize, io::Error> {
    if bytes_with_colon.len() == 1 {
        // Got an empty size value, i.e. ':' with no preceding integers.
        // The header size value cannot be empty, must at least provide a '0:'.
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Header size cannot be an empty string",
        ));
    } else if bytes_with_colon.len() > 2 && bytes_with_colon[0] == b'0' {
        // Size cannot start with a '0' unless it's literally '0:' for empty headers
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Header size cannot be a non-zero value with a leading '0'",
        ));
    }
    // Omit trailing ':' to parse buffer:
    let size_str = String::from_utf8(bytes_with_colon[..bytes_with_colon.len() - 1].to_vec())
        .or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Header size is not a UTF-8 string",
            ))
        })?;
    size_str.parse().or_else(|size_str| {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Header size is not an integer: '{}'", size_str).as_str(),
        ))
    })
}

fn consume_header_string(bytes_with_nul: BytesMut) -> Result<String, io::Error> {
    // Omit trailing NUL to parse buffer as string.
    String::from_utf8(bytes_with_nul[..bytes_with_nul.len() - 1].to_vec()).or_else(|_| {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Header size is not a UTF-8 string",
        ))
    })
}

/// Creates and produces SCGI requests. Invoke once with `Headers`, followed by zero or more calls
/// with `BodyFragment`.
impl Encoder for SCGICodec {
    type Item = SCGIRequest;
    type Error = io::Error;

    fn encode(&mut self, data: SCGIRequest, buf: &mut BytesMut) -> Result<(), io::Error> {
        match data {
            SCGIRequest::Headers(env_map) => {
                if self.encoder_state == CodecState::Content {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "Cannot invoke encoder with Headers after Content (state={:?})",
                            self.encoder_state
                        ),
                    ));
                }
                self.encoder_state = CodecState::Content;

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
                /*if self.encoder_state != CodecState::Content {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Must invoke encoder with Headers before Content",
                    ));
                }*/

                // Forward content as-is
                buf.reserve(fragment.len());
                buf.put(fragment);
            }
        }
        Ok(())
    }
}
