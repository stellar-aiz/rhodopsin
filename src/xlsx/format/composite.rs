//! 日付区分と数値区分を同じ条件選択へ接続する。
use super::{
    DisplayValue, FormatContext, Fragment, NumberFormat, date::DateFormat, decimal::Decimal,
};
use crate::xlsx::diagnostic::{Failure, Result};

pub(super) struct Composite {
    pub(super) selector: NumberFormat,
    dates: Vec<Option<DateFormat>>,
}

fn sections(code: &str) -> Result<Vec<&str>> {
    if code.len() > 4096 {
        return Err(Failure::input("number_format_length"));
    }
    let mut result = Vec::new();
    let (mut start, mut quoted, mut bracket) = (0, false, false);
    let mut chars = code.char_indices();
    while let Some((index, ch)) = chars.next() {
        if quoted {
            if ch == '"' {
                quoted = false;
            }
        } else if bracket {
            if ch == ']' {
                bracket = false;
            }
        } else {
            match ch {
                '"' => quoted = true,
                '[' => bracket = true,
                '\\' | '_' | '*' => {
                    chars
                        .next()
                        .ok_or_else(|| Failure::input("number_format_escape_missing"))?;
                }
                ';' => {
                    result.push(&code[start..index]);
                    start = index + 1;
                }
                _ => {}
            }
        }
    }
    if quoted {
        return Err(Failure::input("number_format_quote_unclosed"));
    }
    if bracket {
        return Err(Failure::input("number_format_bracket_unclosed"));
    }
    result.push(&code[start..]);
    if result.len() > 4 {
        return Err(Failure::input("number_format_section_count"));
    }
    Ok(result)
}

impl Composite {
    pub fn parse(code: &str, context: FormatContext) -> Result<Self> {
        let mut replacements = Vec::new();
        let mut dates = Vec::new();
        for (index, section) in sections(code)?.into_iter().enumerate() {
            if NumberFormat::parse_with_locale(section, context.locale).is_ok() {
                replacements.push(section.to_owned());
                dates.push(None);
                continue;
            }
            if index == 3 {
                return Err(Failure::unsupported("date_text_section"));
            }
            let mut rest = section;
            let mut date_code = String::new();
            let mut conditions = String::new();
            // 日付解析には条件を渡さず、数値区分と同じ条件解析器に渡す。
            while rest.starts_with('[') {
                let end = rest
                    .find(']')
                    .ok_or_else(|| Failure::input("number_format_bracket_unclosed"))?;
                let directive = &rest[1..end];
                if directive.starts_with(['<', '>', '=']) {
                    conditions.push_str(&rest[..=end]);
                } else if super::named_color(directive).is_some()
                    || directive.starts_with('$')
                    || directive.eq_ignore_ascii_case("dbnum3")
                {
                    date_code.push_str(&rest[..=end]);
                } else {
                    break;
                }
                rest = &rest[end + 1..];
            }
            date_code.push_str(rest);
            let date = DateFormat::parse(&date_code, context.locale)?;
            conditions.push('0');
            replacements.push(conditions);
            dates.push(Some(date));
        }
        let selector = NumberFormat::parse_with_locale(&replacements.join(";"), context.locale)?;
        Ok(Self { selector, dates })
    }

    pub fn number(&self, raw: &str, context: FormatContext) -> Result<DisplayValue> {
        let index = self.selector.section_index(&Decimal::parse(raw)?)?;
        match &self.dates[index] {
            Some(date) => Ok(DisplayValue {
                fragments: vec![Fragment::Text(date.format(raw, context.date_system)?)],
                color: date.color().cloned(),
            }),
            None => self.selector.number(raw),
        }
    }

    pub fn text(&self, value: &str) -> Result<DisplayValue> {
        self.selector.text(value)
    }
}
