//! Addresses as a page or a link writes them.

/// `text` with its `%XX` escapes decoded, as a `file:` address writes a
/// space or a letter beyond ASCII; an escape that is no UTF-8 is left as
/// written.
#[must_use]
pub fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%' && index + 3 <= bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        if let Some(byte) = escape {
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::percent_decoded;

    #[test]
    fn escapes_are_decoded() {
        assert_eq!(
            percent_decoded("file:///home/me/my%20notes/%E8%A8%AD%E8%A8%88.md"),
            "file:///home/me/my notes/設計.md"
        );
        assert_eq!(percent_decoded("a%zz%4"), "a%zz%4");
    }
}
