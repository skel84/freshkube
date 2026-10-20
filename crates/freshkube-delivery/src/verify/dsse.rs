//! DSSE's pre-authentication encoding: what a DSSE signature covers.
//! <https://github.com/secure-systems-lab/dsse/blob/master/protocol.md>

/// `"DSSEv1" SP len(type) SP type SP len(body) SP body`, the lengths in
/// decimal bytes, the type and the body as they are (the body is the decoded
/// payload, not its base64).
pub(crate) fn pre_authentication_encoding(payload_type: &str, payload: &[u8]) -> Vec<u8> {
    let mut encoded = format!(
        "DSSEv1 {} {payload_type} {} ",
        payload_type.len(),
        payload.len()
    )
    .into_bytes();
    encoded.extend_from_slice(payload);
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_encoding_is_the_specs_own_example() {
        // The protocol's worked example: type "http://example.com/HelloWorld",
        // body "hello world".
        assert_eq!(
            pre_authentication_encoding("http://example.com/HelloWorld", b"hello world"),
            b"DSSEv1 29 http://example.com/HelloWorld 11 hello world".to_vec()
        );
    }

    #[test]
    fn lengths_count_bytes_not_characters() {
        assert_eq!(
            pre_authentication_encoding("t", "é".as_bytes()),
            b"DSSEv1 1 t 2 \xc3\xa9".to_vec()
        );
    }
}
