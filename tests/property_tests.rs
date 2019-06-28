#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use proptest::prelude::*;
use tokio_codec::{Decoder, Encoder};

use tokio_scgi::client::{SCGICodec as ClientCodec, SCGIRequest as ClientRequest};
use tokio_scgi::server::{SCGICodec as ServerCodec, SCGIRequest as ServerRequest};

#[test]
fn decode_encode_protocol_sample() {
    // Sample from SCGI protocol.txt:
    let protocol_sample = b"70:CONTENT_LENGTH\027\0SCGI\01\0REQUEST_METHOD\0POST\0REQUEST_URI\0/deepthought\0,What is the answer to life?";

    let mut buf = BytesMut::with_capacity(protocol_sample.len());
    buf.put(protocol_sample.to_vec());

    let mut decoder = ServerCodec::new();

    // First call should produce headers
    let mut expected_headers = Vec::new();
    expected_headers.push(("CONTENT_LENGTH".to_string(), "27".to_string()));
    expected_headers.push(("SCGI".to_string(), "1".to_string()));
    expected_headers.push(("REQUEST_METHOD".to_string(), "POST".to_string()));
    expected_headers.push(("REQUEST_URI".to_string(), "/deepthought".to_string()));
    assert_eq!(
        ServerRequest::Headers(expected_headers.clone()),
        decoder.decode(&mut buf).unwrap().unwrap()
    );

    // Second call should produce body
    let expected_body = b"What is the answer to life?";
    assert_eq!(expected_body.len(), buf.len());
    assert_eq!(
        ServerRequest::BodyFragment(expected_body.to_vec()),
        decoder.decode(&mut buf).unwrap().unwrap()
    );

    let mut encoder = ClientCodec::new();
    encoder
        .encode(ClientRequest::Headers(expected_headers), &mut buf)
        .unwrap();
    encoder
        .encode(ClientRequest::BodyFragment(expected_body.to_vec()), &mut buf)
        .unwrap();
    assert_eq!(buf.to_vec(), protocol_sample.to_vec());
}

#[test]
fn encode_decode_empty() {
    check_content(&Vec::new(), &String::new());
}

proptest! {
    #[test]
    fn decode_doesnt_crash(s in ".*") {
        let mut buf = BytesMut::with_capacity(s.len());
        ServerCodec::new().decode(&mut buf)?;
    }

    #[test]
    fn encode_decode_various(headerkey1 in "[^\\x00]+", headerval1 in "[^\\x00]*", headerkey2 in "[^\\x00]+", headerval2 in "[^\\x00]*", content in ".*") {
        let mut headers = Vec::new();
        let empty_content = String::new();

        // no headers (empty content checked above)
        check_content(&headers, &content);

        // one header
        headers.push((headerkey1, headerval1));
        check_content(&headers, &empty_content);
        check_content(&headers, &content);

        // two headers
        headers.push((headerkey2, headerval2));
        check_content(&headers, &empty_content);
        check_content(&headers, &content);
    }
}

fn check_content(headers: &Vec<(String, String)>, content: &String) {
    let mut buf = BytesMut::new();

    let mut encoder = ClientCodec::new();
    encoder.encode(ClientRequest::Headers(headers.clone()), &mut buf).unwrap();
    let content_req = Vec::from(content.as_bytes());
    encoder.encode(ClientRequest::BodyFragment(content_req.clone()), &mut buf).unwrap();

    let encoded_data = buf.clone();

    let mut decoder = ServerCodec::new();
    if let ServerRequest::Headers(headers_decoded) = decoder.decode(&mut buf).unwrap().unwrap() {
        assert_eq!(
            headers, &headers_decoded,
            "headers: {:?} content: {:?} encoded: {:?}",
            headers, content, encoded_data
        );
    } else {
        assert!(false, "expected headers");
    }
    if let ServerRequest::BodyFragment(content_decoded) = decoder.decode(&mut buf).unwrap().unwrap() {
        assert_eq!(
            content_req, content_decoded,
            "headers: {:?} content: {:?} encoded: {:?}",
            headers, content, encoded_data
        );
    } else {
        assert!(false, "expected content");
    }

    check_content_slow(encoded_data, headers, content);
}

/// Run the decoder with byte-by-byte data, then check that the result matches what's expected
fn check_content_slow(
    data: BytesMut,
    expect_headers: &Vec<(String, String)>,
    expect_content: &String,
) {
    let mut buf = BytesMut::with_capacity(data.len());

    let mut got_headers: Vec<(String, String)> = Vec::new();
    let mut got_content = Vec::new();

    // Add each char individually, trying to decode each time:
    let mut decoder = ServerCodec::new();
    for chr in &data {
        buf.put(chr);
        match decoder.decode(&mut buf) {
            Ok(Some(ServerRequest::Headers(headers))) => {
                assert!(
                    got_headers.is_empty(),
                    "Got >1 Headers (added {} from {:?}): prev={:?} this={:?}",
                    chr,
                    data,
                    got_headers,
                    headers
                );
                got_headers.append(&mut headers.clone());
            }
            Ok(Some(ServerRequest::BodyFragment(fragment))) => {
                got_content.append(&mut fragment.clone());
            }
            Ok(None) => {}
            Err(err) => assert!(
                false,
                "Slow content error (added {} from {:?}): {}",
                chr, data, err
            ),
        }
    }

    assert_eq!(expect_headers, &got_headers);
    assert_eq!(expect_content, &String::from_utf8(got_content).unwrap());
}
