#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use proptest::prelude::*;
use tokio_codec::{Decoder, Encoder};

use tokio_scgi::{SCGICodec, SCGIRequest};

#[test]
fn decode_encode_protocol_sample() {
    // Sample from SCGI protocol.txt:
    let protocol_sample = b"70:CONTENT_LENGTH\027\0SCGI\01\0REQUEST_METHOD\0POST\0REQUEST_URI\0/deepthought\0,What is the answer to life?";

    let mut buf = BytesMut::with_capacity(protocol_sample.len());
    buf.put(protocol_sample.to_vec());

    let mut codec = SCGICodec::new();

    // First call should produce headers
    let mut expected_headers = Vec::new();
    expected_headers.push(("CONTENT_LENGTH".to_string(), "27".to_string()));
    expected_headers.push(("SCGI".to_string(), "1".to_string()));
    expected_headers.push(("REQUEST_METHOD".to_string(), "POST".to_string()));
    expected_headers.push(("REQUEST_URI".to_string(), "/deepthought".to_string()));
    assert_eq!(
        SCGIRequest::Headers(expected_headers.clone()),
        codec.decode(&mut buf).unwrap().unwrap()
    );

    // Second call should produce body
    let expected_body = b"What is the answer to life?";
    assert_eq!(expected_body.len(), buf.len());
    assert_eq!(
        SCGIRequest::BodyFragment(expected_body.to_vec()),
        codec.decode(&mut buf).unwrap().unwrap()
    );

    codec
        .encode(SCGIRequest::Headers(expected_headers), &mut buf)
        .unwrap();
    codec
        .encode(SCGIRequest::BodyFragment(expected_body.to_vec()), &mut buf)
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
        let mut codec = SCGICodec::new();
        codec.decode(&mut buf)?;
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
    let mut codec = SCGICodec::new();
    let mut buf = BytesMut::new();

    let headers_req = SCGIRequest::Headers(headers.clone());
    codec.encode(headers_req.clone(), &mut buf).unwrap();
    let content_req = SCGIRequest::BodyFragment(Vec::from(content.as_bytes()));
    codec.encode(content_req.clone(), &mut buf).unwrap();

    let encoded_data = buf.clone();

    let headers_decoded = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(
        headers_req, headers_decoded,
        "headers: {:?} content: {:?} encoded: {:?}",
        headers, content, encoded_data
    );
    let content_decoded = codec.decode(&mut buf).unwrap().unwrap();
    assert_eq!(
        content_req, content_decoded,
        "headers: {:?} content: {:?} encoded: {:?}",
        headers, content, encoded_data
    );

    check_content_slow(encoded_data, headers, content);
}

/// Run the decoder with byte-by-byte data, then check that the result matches what's expected
fn check_content_slow(
    data: BytesMut,
    expect_headers: &Vec<(String, String)>,
    expect_content: &String,
) {
    let mut codec = SCGICodec::new();
    let mut buf = BytesMut::with_capacity(data.len());

    let mut got_headers: Vec<(String, String)> = Vec::new();
    let mut got_content = Vec::new();

    // Add each char individually, trying to decode each time:
    for chr in &data {
        buf.put(chr);
        match codec.decode(&mut buf) {
            Ok(Some(SCGIRequest::Headers(headers))) => {
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
            Ok(Some(SCGIRequest::BodyFragment(fragment))) => {
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
