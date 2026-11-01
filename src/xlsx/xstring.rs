//! SpreadsheetML の ST_Xstring。XML エンティティの復号後に一度だけ適用する。
use crate::xlsx::diagnostic::{Failure, Result};

pub fn decode(value: &str) -> Result<String> {
    let mut units = Vec::with_capacity(value.len());
    let mut remaining = value;
    while !remaining.is_empty() {
        let bytes = remaining.as_bytes();
        if bytes.len() >= 7
            && bytes[0] == b'_'
            && bytes[1] == b'x'
            && bytes[6] == b'_'
            && bytes[2..6].iter().all(u8::is_ascii_hexdigit)
        {
            let code = u16::from_str_radix(&remaining[2..6], 16)
                .map_err(|_| Failure::input("invalid_string_escape"))?;
            units.push(code);
            remaining = &remaining[7..];
        } else {
            let ch = remaining
                .chars()
                .next()
                .ok_or_else(|| Failure::input("invalid_string_escape"))?;
            let mut encoded = [0; 2];
            units.extend_from_slice(ch.encode_utf16(&mut encoded));
            remaining = &remaining[ch.len_utf8()..];
        }
    }
    String::from_utf16(&units).map_err(|_| Failure::input("invalid_string_surrogate"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn エスケープを一度だけ復号する() {
        assert_eq!(decode("見積_x000D_書").unwrap(), "見積\r書");
        assert_eq!(decode("_x005F_x0041_").unwrap(), "_x0041_");
        assert_eq!(decode("_x005f_x005F_").unwrap(), "_x005F_");
        assert_eq!(decode("_xD83D__xDE00_").unwrap(), "😀");
        assert_eq!(decode("😀_x0041_見").unwrap(), "😀A見");
        assert_eq!(decode("_xZZZZ__x12_末尾_").unwrap(), "_xZZZZ__x12_末尾_");
    }
    #[test]
    fn 不正なサロゲートを置換文字で隠さない() {
        for value in ["_xD800_", "_xDC00_", "_xD800_A", "_xD800__xD800_機密"] {
            let error = decode(value).unwrap_err();
            assert_eq!(error.reason, "invalid_string_surrogate");
            assert!(!serde_json::to_string(&error).unwrap().contains("機密"));
        }
    }
}
