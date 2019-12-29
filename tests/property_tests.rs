#![deny(warnings, rust_2018_idioms)]

use bytes::{BufMut, BytesMut};
use proptest::prelude::*;
use tokio_util::codec::{Decoder, Encoder};

use tokio_scgi::client::{SCGICodec as ClientCodec, SCGIRequest as ClientRequest};
use tokio_scgi::server::{SCGICodec as ServerCodec, SCGIRequest as ServerRequest};

#[test]
fn decode_encode_protocol_sample() {
    // Sample from SCGI protocol.txt:
    let protocol_sample = b"70:CONTENT_LENGTH\027\0SCGI\01\0REQUEST_METHOD\0POST\0REQUEST_URI\0/deepthought\0,What is the answer to life?";

    let mut buf = BytesMut::with_capacity(protocol_sample.len());
    buf.put_slice(protocol_sample);

    let mut decoder = ServerCodec::new();

    // First call should produce both headers and body
    let mut expected_headers = Vec::new();
    expected_headers.push(("CONTENT_LENGTH".to_string(), "27".to_string()));
    expected_headers.push(("SCGI".to_string(), "1".to_string()));
    expected_headers.push(("REQUEST_METHOD".to_string(), "POST".to_string()));
    expected_headers.push(("REQUEST_URI".to_string(), "/deepthought".to_string()));
    let expected_body_str = b"What is the answer to life?";
    let mut expected_body = BytesMut::new();
    expected_body.reserve(expected_body_str.len());
    expected_body.put_slice(expected_body_str);
    assert_eq!(
        ServerRequest::Request(expected_headers.clone(), expected_body.clone()),
        decoder.decode(&mut buf).unwrap().unwrap()
    );

    // Encoding meanwhile should get us back to the sample data. First try headers+body:
    let mut encoder = ClientCodec::new();
    encoder
        .encode(
            ClientRequest::Request(expected_headers.clone(), expected_body.clone()),
            &mut buf,
        )
        .unwrap();
    assert_eq!(buf.to_vec(), protocol_sample.to_vec());
    buf.clear();

    // Then try again with headers and body in separate calls:
    encoder
        .encode(
            ClientRequest::Request(expected_headers, BytesMut::new()),
            &mut buf,
        )
        .unwrap();
    encoder
        .encode(ClientRequest::BodyFragment(expected_body.clone()), &mut buf)
        .unwrap();
    assert_eq!(buf.to_vec(), protocol_sample.to_vec());
}

#[test]
fn encode_decode_empty_headers() {
    let mut buf = BytesMut::new();

    // First send empty headers.
    ClientCodec::new()
        .encode(
            ClientRequest::Request(Vec::new(), BytesMut::new()),
            &mut buf,
        )
        .unwrap();
    assert_eq!("0:,".as_bytes(), buf);

    // Should get empty headers back too
    if let ServerRequest::Request(headers, body) =
        ServerCodec::new().decode(&mut buf).unwrap().unwrap()
    {
        assert_eq!(0, headers.len());
        assert_eq!(0, body.len());
    } else {
        assert!(false, "expected None");
    }

    check_content_slow(buf, Vec::new(), &String::new());
}

#[test]
fn encode_decode_empty_body() {
    let mut buf = BytesMut::new();

    // First send empty data.
    ClientCodec::new()
        .encode(ClientRequest::BodyFragment(BytesMut::new()), &mut buf)
        .unwrap();
    assert_eq!(0, buf.len());

    // Should get None when nothing's left
    if let None = ServerCodec::new().decode(&mut buf).unwrap() {
    } else {
        assert!(false, "expected None");
    }

    check_content_slow(buf, Vec::new(), &String::new());
}

proptest! {
    #[test]
    fn server_decode_doesnt_crash(s in ".*") {
        let mut buf = BytesMut::from(s.as_bytes());
        // ignore any io errors, they're expected
        let _ = ServerCodec::new().decode(&mut buf);
    }

    #[test]
    fn client_decode_doesnt_crash(s in ".*") {
        let mut buf = BytesMut::from(s.as_bytes());
        // ignore any io errors, they're expected
        let _ = ClientCodec::new().decode(&mut buf)?;
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
    let content_req = BytesMut::from(content.as_bytes());

    // First send the headers+content in a single request

    encoder
        .encode(
            ClientRequest::Request(headers.clone(), content_req.clone()),
            &mut buf,
        )
        .unwrap();
    let encoded_data_combined = buf.clone();

    let mut decoder = ServerCodec::new();
    if let ServerRequest::Request(headers_decoded, body_decoded) =
        decoder.decode(&mut buf).unwrap().unwrap()
    {
        assert_eq!(headers, &headers_decoded);
        assert_eq!(content_req, body_decoded);
    } else {
        assert!(false, "expected Headers (with content)");
    }

    // Should get None when nothing's left
    assert_eq!(0, buf.len());
    if let None = decoder.decode(&mut buf).unwrap() {
    } else {
        assert!(false, "expected None");
    }

    check_content_slow(encoded_data_combined, headers.to_vec(), content);

    // Then do it again with just the headers and empty content

    encoder
        .encode(
            ClientRequest::Request(headers.clone(), BytesMut::new()),
            &mut buf,
        )
        .unwrap();
    let encoded_data_header_only = buf.clone();

    let mut decoder = ServerCodec::new();
    if let ServerRequest::Request(headers_decoded, body_decoded) =
        decoder.decode(&mut buf).unwrap().unwrap()
    {
        assert_eq!(headers, &headers_decoded);
        assert_eq!(0, body_decoded.len());
    } else {
        assert!(false, "expected Headers (without content)");
    }

    // Should get None when nothing's left
    assert_eq!(0, buf.len());
    if let None = decoder.decode(&mut buf).unwrap() {
    } else {
        assert!(false, "expected None");
    }

    check_content_slow(encoded_data_header_only, headers.clone(), &String::new());

    // Finally try the headers+content as separate payloads

    encoder
        .encode(
            ClientRequest::Request(headers.clone(), BytesMut::new()),
            &mut buf,
        )
        .unwrap();
    encoder
        .encode(ClientRequest::BodyFragment(content_req.clone()), &mut buf)
        .unwrap();
    let encoded_data_separate = buf.clone();

    let mut decoder = ServerCodec::new();
    let r = decoder.decode(&mut buf).unwrap().unwrap();
    if let ServerRequest::Request(headers_decoded, body_decoded) = r {
        assert_eq!(headers, &headers_decoded);
        assert_eq!(content_req, body_decoded);
    } else {
        assert!(
            false,
            "expected Headers (with content): {:?} (from {:?})",
            r, encoded_data_separate
        );
    }

    // Should get None when nothing's left
    assert_eq!(0, buf.len());
    if let None = decoder.decode(&mut buf).unwrap() {
    } else {
        assert!(false, "expected None");
    }

    check_content_slow(encoded_data_separate, headers.clone(), content);
}

/// Run the decoder with byte-by-byte data, then check that the result matches what's expected
fn check_content_slow(
    data: BytesMut,
    expect_headers: Vec<(String, String)>,
    expect_content: &String,
) {
    let mut buf = BytesMut::with_capacity(data.len());

    let mut got_headers: Vec<(String, String)> = Vec::new();
    let mut got_content = BytesMut::new();

    // Add each char individually, trying to decode each time:
    let mut decoder = ServerCodec::new();
    for chr in &data {
        buf.put_u8(*chr);
        match decoder.decode(&mut buf) {
            Ok(Some(ServerRequest::Request(headers, body))) => {
                assert!(
                    got_headers.is_empty(),
                    "Got >1 Headers (added {} from {:?}): prev={:?} this={:?}",
                    chr,
                    data,
                    got_headers,
                    headers
                );
                got_headers.append(&mut headers.clone());
                got_content.reserve(body.len());
                got_content.put(body);
            }
            Ok(Some(ServerRequest::BodyFragment(fragment))) => {
                got_content.reserve(fragment.len());
                got_content.put(fragment);
            }
            Ok(None) => {}
            Err(err) => assert!(
                false,
                "Slow content error (added {} from {:?}): {}",
                chr, data, err
            ),
        }
    }

    let got_content_str = String::from_utf8(got_content.to_vec()).unwrap();
    assert_eq!(expect_headers, got_headers);
    assert_eq!(
        expect_content, &got_content_str,
        "left: {:?} right: {:?}",
        expect_content, got_content_str
    );
}
