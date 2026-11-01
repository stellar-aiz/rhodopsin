//! 日本語の桁ごとの数字表記。万・億などの単位を伴う General は別の規則。
use crate::xlsx::diagnostic::{Failure, Result};

#[derive(Clone, Copy, Debug)]
pub(super) enum NativeDigits {
    Kanji,
    Formal,
    FullWidth,
}
impl NativeDigits {
    pub fn parse(directive: &str) -> Result<Self> {
        match directive.to_ascii_lowercase().as_str() {
            "dbnum1" => Ok(Self::Kanji),
            "dbnum2" => Ok(Self::Formal),
            "dbnum3" => Ok(Self::FullWidth),
            _ => Err(Failure::unsupported("number_format_dbnum")),
        }
    }
    pub fn convert(self, digits: &str) -> Result<String> {
        let alphabet = match self {
            Self::Kanji => ['〇', '一', '二', '三', '四', '五', '六', '七', '八', '九'],
            Self::Formal => ['零', '壱', '弐', '参', '四', '伍', '六', '七', '八', '九'],
            Self::FullWidth => ['０', '１', '２', '３', '４', '５', '６', '７', '８', '９'],
        };
        digits
            .bytes()
            .map(|digit| {
                if digit.is_ascii_digit() {
                    Ok(alphabet[usize::from(digit - b'0')])
                } else {
                    Err(Failure::unsupported("dbnum_numeric_pattern"))
                }
            })
            .collect()
    }
}
