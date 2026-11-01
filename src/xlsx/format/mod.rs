mod composite;
mod condition;
pub mod date;
mod decimal;
mod fraction;
pub(crate) mod general;
mod native_digits;
use crate::xlsx::{
    diagnostic::{Failure, Result},
    styles::{Color, ColorSource},
};
use decimal::Decimal;

#[derive(Clone, Debug, PartialEq)]
pub enum Fragment {
    Text(String),
    /// この文字を描かず、その実フォント幅だけ進める。
    Space(char),
    /// 残った幅をこの文字の繰り返しで埋める。配置時に回数を決める。
    Fill(char),
}
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayValue {
    pub fragments: Vec<Fragment>,
    pub color: Option<Color>,
}
#[derive(Clone, Debug)]
enum Token {
    Digit(char),
    Dot,
    Slash,
    Comma,
    Percent,
    Exponent(char, bool),
    Text(String),
    At,
    Space(char),
    Fill(char),
}
#[derive(Clone, Debug, Default)]
struct Section {
    tokens: Vec<Token>,
    color: Option<Color>,
    locale_present: bool,
    japanese_locale: bool,
    native_digits: Option<native_digits::NativeDigits>,
    condition: Option<condition::Condition>,
}
#[derive(Clone, Debug)]
pub struct NumberFormat {
    sections: Vec<Section>,
}

fn push_text(out: &mut Vec<Fragment>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(Fragment::Text(previous)) = out.last_mut() {
        previous.push_str(text);
    } else {
        out.push(Fragment::Text(text.into()));
    }
}

impl NumberFormat {
    pub fn parse(code: &str) -> Result<Self> {
        Self::parse_internal(code, None)
    }

    /// OS の設定を参照せず、呼び出し元が明示したロケールを使う。
    /// 書式区分自身のロケール指定があれば、そちらを優先する。
    pub fn parse_with_locale(code: &str, locale: date::Locale) -> Result<Self> {
        Self::parse_internal(code, Some(locale))
    }

    fn parse_internal(code: &str, locale: Option<date::Locale>) -> Result<Self> {
        if code.len() > 4096 {
            return Err(Failure::input("number_format_length"));
        }
        if code.eq_ignore_ascii_case("general") {
            return Err(Failure::unsupported("general_number_layout"));
        }
        let mut sections = vec![Section::default()];
        let mut chars = code.chars();
        while let Some(c) = chars.next() {
            if c == ';' {
                if sections.len() == 4 {
                    return Err(Failure::input("number_format_section_count"));
                }
                sections.push(Section::default());
                continue;
            }
            let section = sections
                .last_mut()
                .ok_or_else(|| Failure::input("number_format_section_count"))?;
            let token = match c {
                '0' | '#' | '?' => Token::Digit(c),
                '.' => Token::Dot,
                '/' => Token::Slash,
                ',' => Token::Comma,
                '%' => Token::Percent,
                '@' => Token::At,
                'E' | 'e' => {
                    let positive_sign = match chars.next() {
                        Some('+') => true,
                        Some('-') => false,
                        _ => return Err(Failure::unsupported("number_format_exponent_sign")),
                    };
                    Token::Exponent(c, positive_sign)
                }
                '_' | '*' => {
                    let next = chars
                        .next()
                        .ok_or_else(|| Failure::input("number_format_escape_missing"))?;
                    if c == '_' {
                        Token::Space(next)
                    } else {
                        Token::Fill(next)
                    }
                }
                '\\' => Token::Text(
                    chars
                        .next()
                        .ok_or_else(|| Failure::input("number_format_escape_missing"))?
                        .to_string(),
                ),
                '"' => {
                    let mut literal = String::new();
                    let mut closed = false;
                    for ch in chars.by_ref() {
                        if ch == '"' {
                            closed = true;
                            break;
                        }
                        literal.push(ch);
                    }
                    if !closed {
                        return Err(Failure::input("number_format_quote_unclosed"));
                    }
                    Token::Text(literal)
                }
                '[' => {
                    let mut directive = String::new();
                    let mut closed = false;
                    for ch in chars.by_ref() {
                        if ch == ']' {
                            closed = true;
                            break;
                        }
                        directive.push(ch);
                    }
                    if !closed {
                        return Err(Failure::input("number_format_bracket_unclosed"));
                    }
                    if directive.to_ascii_lowercase().starts_with("dbnum") {
                        if section.native_digits.is_some()
                            || section
                                .tokens
                                .iter()
                                .any(|t| !matches!(t, Token::Text(s) if s.is_empty()))
                        {
                            return Err(Failure::input("number_format_dbnum_position"));
                        }
                        section.native_digits =
                            Some(native_digits::NativeDigits::parse(&directive)?);
                        continue;
                    }
                    if let Some(color) = named_color(&directive) {
                        if section.color.is_some() || !section.tokens.is_empty() {
                            return Err(Failure::input("number_format_color_position"));
                        }
                        section.color = Some(color);
                        continue;
                    }
                    if directive.starts_with(['<', '>', '=']) {
                        if section.condition.is_some() || !section.tokens.is_empty() {
                            return Err(Failure::input("number_format_condition_position"));
                        }
                        section.condition = Some(condition::Condition::parse(&directive)?);
                        continue;
                    }
                    if let Some(currency) = directive.strip_prefix('$') {
                        if section.locale_present {
                            return Err(Failure::input("duplicate_number_format_locale"));
                        }
                        section.locale_present = true;
                        section.japanese_locale = currency
                            .strip_prefix('-')
                            .and_then(date::Locale::from_identifier)
                            == Some(date::Locale::Japanese);
                        Token::Text(currency_symbol(currency)?)
                    } else {
                        return Err(Failure::unsupported("number_format_directive"));
                    }
                }
                c if c.is_alphabetic() || c == '/' || c == ':' => {
                    return Err(Failure::unsupported("number_format_date_or_exponent"));
                }
                c => Token::Text(c.to_string()),
            };
            section.tokens.push(token);
        }
        for section in &sections {
            if section.native_digits.is_some() {
                let japanese = if section.locale_present {
                    section.japanese_locale
                } else {
                    locale == Some(date::Locale::Japanese)
                };
                if !japanese {
                    return Err(Failure::unsupported("dbnum_locale"));
                }
                if !section.tokens.iter().any(|t| matches!(t, Token::Digit(_)))
                    || section
                        .tokens
                        .iter()
                        .any(|t| !matches!(t, Token::Digit('0' | '#') | Token::Text(_)))
                {
                    return Err(Failure::unsupported("dbnum_numeric_pattern"));
                }
            }
            if section
                .tokens
                .iter()
                .filter(|t| matches!(t, Token::Fill(_)))
                .count()
                > 1
            {
                return Err(Failure::unsupported("multiple_number_format_fills"));
            }
        }
        if sections.iter().skip(2).any(|s| s.condition.is_some()) {
            return Err(Failure::input("number_format_condition_position"));
        }
        if sections.get(1).is_some_and(|s| s.condition.is_some()) && sections[0].condition.is_none()
        {
            return Err(Failure::unsupported("number_format_condition_layout"));
        }
        if sections[0].condition.is_some() && sections.len() >= 3 && sections[1].condition.is_none()
        {
            return Err(Failure::unsupported("number_format_condition_layout"));
        }
        Ok(Self { sections })
    }

    pub fn number(&self, raw: &str) -> Result<DisplayValue> {
        let value = Decimal::parse(raw)?;
        let conditional = self.sections[0].condition.is_some();
        let index = self.section_index(&value)?;
        let section = &self.sections[index];
        if value.negative && section.native_digits.is_some() {
            return Err(Failure::unsupported("dbnum_negative_format"));
        }
        if section.tokens.iter().any(|t| matches!(t, Token::At)) {
            return Err(Failure::unsupported("text_format_numeric_value"));
        }
        let mut fragments = Vec::new();
        if value.negative
            && (self.sections.len() == 1 || conditional && index != 1)
            && !section.tokens.is_empty()
        {
            push_text(&mut fragments, "-");
        }
        self.render_number(&value, section, fragments)
    }

    fn section_index(&self, value: &Decimal) -> Result<usize> {
        Ok(if self.sections[0].condition.is_some() {
            self.sections
                .iter()
                .take(3)
                .position(|section| section.condition.as_ref().is_none_or(|c| c.matches(value)))
                .ok_or_else(|| Failure::unsupported("number_format_condition_unmatched"))?
        } else if value.is_zero() && self.sections.len() >= 3 {
            2
        } else if value.negative && self.sections.len() >= 2 {
            1
        } else {
            0
        })
    }

    fn render_number(
        &self,
        value: &Decimal,
        section: &Section,
        mut fragments: Vec<Fragment>,
    ) -> Result<DisplayValue> {
        let first = section
            .tokens
            .iter()
            .position(|t| matches!(t, Token::Digit(_)));
        let last = section
            .tokens
            .iter()
            .rposition(|t| matches!(t, Token::Digit(_)));
        let percent = section
            .tokens
            .iter()
            .filter(|t| matches!(t, Token::Percent))
            .count();
        match (first, last) {
            (Some(mut first), Some(mut last)) => {
                // .00 と 0. の小数点も数値パターンの一部。引用された点は Text のまま。
                if first > 0 && matches!(section.tokens[first - 1], Token::Dot) {
                    first -= 1;
                }
                if matches!(section.tokens.get(last + 1), Some(Token::Dot)) {
                    last += 1;
                }
                let mut end = last + 1;
                while matches!(section.tokens.get(end), Some(Token::Comma)) {
                    end += 1;
                }
                let scale = (end - last - 1) as i32;
                for token in &section.tokens[..first] {
                    emit(token, None, &mut fragments)?;
                }
                let mut digits = Vec::new();
                numeric(
                    value,
                    &section.tokens[first..=last],
                    percent as i32 * 2 - scale * 3,
                    &mut digits,
                )?;
                for fragment in digits {
                    match fragment {
                        Fragment::Text(value) => {
                            let value = if let Some(native) = section.native_digits {
                                native.convert(&value)?
                            } else {
                                value
                            };
                            push_text(&mut fragments, &value);
                        }
                        other => fragments.push(other),
                    }
                }
                for token in &section.tokens[end..] {
                    emit(token, None, &mut fragments)?;
                }
            }
            _ => {
                for token in &section.tokens {
                    emit(token, None, &mut fragments)?;
                }
            }
        }
        Ok(DisplayValue {
            fragments,
            color: section.color.clone(),
        })
    }

    pub fn text(&self, value: &str) -> Result<DisplayValue> {
        let section = self.text_section();
        let Some(section) = section else {
            return Ok(DisplayValue {
                fragments: vec![Fragment::Text(value.into())],
                color: None,
            });
        };
        let mut fragments = Vec::new();
        for token in &section.tokens {
            emit(token, Some(value), &mut fragments)?;
        }
        Ok(DisplayValue {
            fragments,
            color: section.color.clone(),
        })
    }

    fn text_section(&self) -> Option<&Section> {
        if self.sections.len() == 4 {
            Some(&self.sections[3])
        } else if self.sections.len() == 1
            && self.sections[0]
                .tokens
                .iter()
                .any(|t| matches!(t, Token::At))
        {
            Some(&self.sections[0])
        } else {
            None
        }
    }
}

// この二つのロケールは、このエンジンの小数点・桁区切りと一致する。
fn currency_symbol(value: &str) -> Result<String> {
    let lower = value.to_ascii_lowercase();
    let symbol = if lower.ends_with("-ja-jp") || lower.ends_with("-en-us") {
        &value[..value.len() - 6]
    } else if let Some((symbol, locale)) = value.rsplit_once('-') {
        if !(3..=8).contains(&locale.len()) || !locale.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(Failure::input("invalid_number_format_locale"));
        }
        if date::Locale::from_identifier(locale).is_none() {
            return Err(Failure::unsupported("number_format_locale"));
        }
        symbol
    } else {
        value
    };
    if symbol
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '[' | ']' | '"'))
    {
        return Err(Failure::input("invalid_currency_symbol"));
    }
    Ok(symbol.into())
}

fn named_color(value: &str) -> Option<Color> {
    let rgb = match value.to_ascii_lowercase().as_str() {
        "black" => [0, 0, 0],
        "blue" => [0, 0, 255],
        "cyan" => [0, 255, 255],
        "green" => [0, 255, 0],
        "magenta" => [255, 0, 255],
        "red" => [255, 0, 0],
        "white" => [255, 255, 255],
        "yellow" => [255, 255, 0],
        _ => return None,
    };
    Some(Color {
        source: ColorSource::Rgb(rgb),
        tint: 0.0,
    })
}
fn emit(token: &Token, text: Option<&str>, out: &mut Vec<Fragment>) -> Result<()> {
    match token {
        Token::Text(s) => push_text(out, s),
        Token::Percent => push_text(out, "%"),
        Token::Space(c) => out.push(Fragment::Space(*c)),
        Token::Fill(c) => out.push(Fragment::Fill(*c)),
        Token::At => {
            if let Some(text) = text {
                push_text(out, text);
            }
        }
        _ => return Err(Failure::unsupported("number_format_token_position")),
    }
    Ok(())
}

fn numeric(
    value: &Decimal,
    tokens: &[Token],
    mut shift: i32,
    out: &mut Vec<Fragment>,
) -> Result<()> {
    if let Some(index) = tokens
        .iter()
        .position(|token| matches!(token, Token::Exponent(_, _)))
    {
        return scientific(value, tokens, index, shift, out);
    }
    if tokens.iter().any(|token| matches!(token, Token::Slash)) {
        return fraction::render(value, tokens, shift, out);
    }
    let mut integer = Vec::new();
    let mut fraction = Vec::new();
    let mut decimal = false;
    let mut grouping = false;
    let dot = tokens
        .iter()
        .position(|t| matches!(t, Token::Dot))
        .unwrap_or(tokens.len());
    let last_integer = tokens[..dot]
        .iter()
        .rposition(|t| matches!(t, Token::Digit(_)));
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::Digit(c) => {
                if decimal {
                    fraction.push(*c);
                } else {
                    integer.push(*c);
                }
            }
            Token::Dot if !decimal => decimal = true,
            Token::Comma if !decimal => {
                if last_integer.is_some_and(|last| index > last) {
                    shift -= 3;
                } else {
                    grouping = true;
                }
            }
            _ => return Err(Failure::unsupported("number_format_numeric_pattern")),
        }
    }
    let (mut whole, decimals) = value.fixed(fraction.len(), shift)?;
    let minimum = integer.iter().filter(|c| **c == '0').count();
    if whole == "0" && minimum == 0 {
        whole.clear();
    }
    if whole.len() < minimum {
        whole = format!("{}{}", "0".repeat(minimum - whole.len()), whole);
    }
    let missing = integer.len().saturating_sub(whole.len());
    for c in integer.iter().take(missing) {
        if *c == '?' {
            out.push(Fragment::Space('0'));
        }
    }
    if grouping && !whole.is_empty() {
        let mut grouped = String::new();
        for (index, c) in whole.chars().enumerate() {
            if index != 0 && (whole.len() - index) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        whole = grouped;
    }
    push_text(out, &whole);
    if decimal {
        push_text(out, ".");
    }
    let significant = decimals
        .bytes()
        .rposition(|c| c != b'0')
        .map_or(0, |i| i + 1);
    let mandatory = fraction
        .iter()
        .rposition(|c| *c == '0')
        .map_or(0, |i| i + 1);
    for (index, (pattern, digit)) in fraction.iter().zip(decimals.chars()).enumerate() {
        if index < significant.max(mandatory) {
            push_text(out, &digit.to_string());
        } else if *pattern == '?' {
            out.push(Fragment::Space('0'));
        }
    }
    Ok(())
}

fn scientific(
    value: &Decimal,
    tokens: &[Token],
    index: usize,
    shift: i32,
    out: &mut Vec<Fragment>,
) -> Result<()> {
    let mantissa = &tokens[..index];
    let exponent = &tokens[index + 1..];
    let dot = mantissa
        .iter()
        .position(|token| matches!(token, Token::Dot))
        .unwrap_or(mantissa.len());
    let engineering = matches!(
        mantissa.get(..dot),
        Some([Token::Digit('#'), Token::Digit('#'), Token::Digit('0')])
    );
    let standard = dot == 1 && matches!(mantissa.first(), Some(Token::Digit('0')));
    // 必須桁が複数の仮数・百分率や桁区切りとの組合せは個別対応する。
    if shift != 0
        || !(standard || engineering)
        || mantissa
            .iter()
            .skip(dot + 1)
            .any(|token| !matches!(token, Token::Digit(_)))
        || exponent.is_empty()
        || exponent.len() > 30
        || exponent
            .iter()
            .any(|token| !matches!(token, Token::Digit('0')))
    {
        return Err(Failure::unsupported("scientific_number_pattern"));
    }
    let places = if dot < mantissa.len() {
        mantissa.len() - dot - 1
    } else {
        0
    };
    let (normalized, power) = if engineering {
        value.scientific_group(places, 3)?
    } else {
        value.scientific(places)?
    };
    numeric(&normalized, mantissa, 0, out)?;
    let Token::Exponent(letter, positive_sign) = tokens[index] else {
        unreachable!()
    };
    push_text(out, &letter.to_string());
    if power < 0 {
        push_text(out, "-");
    } else if positive_sign {
        push_text(out, "+");
    }
    push_text(
        out,
        &format!("{:0width$}", power.unsigned_abs(), width = exponent.len()),
    );
    Ok(())
}

/// ロケールに依存しない組み込み形式。日付・会計・標準は別処理を追加する。
pub fn builtin(id: u32) -> Result<&'static str> {
    match id {
        0 => Ok("General"),
        1 => Ok("0"),
        2 => Ok("0.00"),
        3 => Ok("#,##0"),
        4 => Ok("#,##0.00"),
        9 => Ok("0%"),
        10 => Ok("0.00%"),
        11 => Ok("0.00E+00"),
        12 => Ok("# ?/?"),
        13 => Ok("# ??/??"),
        // OOXML の表と Excel の実装差を反映し、正数の括弧幅は空白で確保する。
        37 => Ok("#,##0_);(#,##0)"),
        38 => Ok("#,##0_);[Red](#,##0)"),
        39 => Ok("#,##0.00_);(#,##0.00)"),
        40 => Ok("#,##0.00_);[Red](#,##0.00)"),
        48 => Ok("##0.0E+0"),
        49 => Ok("@"),
        5..=8 | 23..=26 | 41..=44 | 63..=66 => {
            Err(Failure::unsupported("locale_number_format_missing"))
        }
        _ => Err(Failure::unsupported("builtin_number_format")),
    }
}

/// セルと styles.xml の表示形式参照を接続する。診断にはセル値を含めない。
pub fn cell(
    value: &crate::xlsx::workbook::Cell,
    styles: Option<&crate::xlsx::styles::Styles>,
) -> Result<DisplayValue> {
    cell_impl(value, styles, None)
}

#[derive(Clone, Copy, Debug)]
pub struct FormatContext {
    pub date_system: date::DateSystem,
    pub locale: date::Locale,
}

/// 組み込みの日付形式は Excel の表示ロケールを明示して解決する。
/// English は en-US。OS の地域設定は参照しない。
pub fn builtin_with_context(id: u32, context: FormatContext) -> Result<&'static str> {
    if matches!(id, 27..=36 | 50..=58) {
        if context.locale != date::Locale::Japanese {
            return Err(Failure::unsupported("builtin_date_locale"));
        }
        // 同じ番号でも言語によって形式が異なるため、日本語指定時だけ解決する。
        return Ok(match id {
            27 | 36 | 50 | 57 => "[$-411]ge.m.d",
            28 | 29 | 51 | 54 | 58 => "[$-411]ggge\"年\"m\"月\"d\"日\"",
            30 => "m/d/yy",
            31 => "yyyy\"年\"m\"月\"d\"日\"",
            32 => "h\"時\"mm\"分\"",
            33 => "h\"時\"mm\"分\"ss\"秒\"",
            34 | 52 | 55 => "yyyy\"年\"m\"月\"",
            35 | 53 | 56 => "m\"月\"d\"日\"",
            _ => unreachable!(),
        });
    }
    if matches!(id, 14..=22 | 45..=47) {
        if context.locale != date::Locale::English {
            return Err(Failure::unsupported("builtin_date_locale"));
        }
        return Ok(match id {
            // ISO の表ではなく MS-OI29500 に記載された Excel の形式。
            14 => "m/d/yyyy",
            15 => "d-mmm-yy",
            16 => "d-mmm",
            17 => "mmm-yy",
            18 => "h:mm AM/PM",
            19 => "h:mm:ss AM/PM",
            20 => "h:mm",
            21 => "h:mm:ss",
            22 => "m/d/yyyy h:mm",
            45 => "mm:ss",
            46 => "[h]:mm:ss",
            47 => "mm:ss.0",
            _ => unreachable!(),
        });
    }
    builtin(id)
}

pub fn cell_with_context(
    value: &crate::xlsx::workbook::Cell,
    styles: Option<&crate::xlsx::styles::Styles>,
    context: FormatContext,
) -> Result<DisplayValue> {
    cell_impl(value, styles, Some(context))
}

fn cell_impl(
    value: &crate::xlsx::workbook::Cell,
    styles: Option<&crate::xlsx::styles::Styles>,
    context: Option<FormatContext>,
) -> Result<DisplayValue> {
    use crate::xlsx::workbook::CellValue;
    let plain = |s: String| DisplayValue {
        fragments: vec![Fragment::Text(s)],
        color: None,
    };
    if matches!(value.value, CellValue::Blank) {
        return Ok(DisplayValue {
            fragments: Vec::new(),
            color: None,
        });
    }
    if matches!(value.value, CellValue::RichText(_)) {
        return Err(Failure::unsupported("rich_text_requires_run_layout"));
    }
    let id = styles
        .map(|s| s.validate_cell(value.style).map(|s| s.number_format))
        .transpose()?
        .unwrap_or(0);
    let code = match styles.and_then(|s| s.number_formats.get(&id)) {
        Some(code) => code.as_str(),
        None => match context {
            Some(context) => builtin_with_context(id, context)?,
            None => builtin(id)?,
        },
    };
    // 非数値のキャッシュは数値 0/1 や通常文字列へ型変換しない。
    // カスタム区分の適用規則は Excel fixture で確認してから追加する。
    match &value.value {
        CellValue::Boolean(boolean) => {
            if !code.eq_ignore_ascii_case("General") {
                return Err(Failure::unsupported("boolean_custom_format"));
            }
            return Ok(plain(if *boolean { "TRUE" } else { "FALSE" }.into()));
        }
        CellValue::Error(error) => {
            if !code.eq_ignore_ascii_case("General") {
                return Err(Failure::unsupported("error_custom_format"));
            }
            if !matches!(
                error.as_str(),
                "#NULL!"
                    | "#DIV/0!"
                    | "#VALUE!"
                    | "#REF!"
                    | "#NAME?"
                    | "#NUM!"
                    | "#N/A"
                    | "#GETTING_DATA"
            ) {
                return Err(Failure::unsupported("unknown_cell_error"));
            }
            return Ok(plain(error.clone()));
        }
        _ => {}
    }
    if let CellValue::Text(text) = &value.value
        && code.eq_ignore_ascii_case("General")
    {
        return Ok(plain(text.to_string()));
    }
    let parsed = match context {
        Some(context) => NumberFormat::parse_with_locale(code, context.locale),
        None => NumberFormat::parse(code),
    };
    let formatter = match parsed {
        Ok(formatter) => formatter,
        Err(error) => {
            if let Some(context) = context
                && let Ok(formatter) = composite::Composite::parse(code, context)
            {
                return match &value.value {
                    CellValue::Number(_) => formatter.number(
                        value.numeric_lexeme.as_deref().ok_or_else(|| {
                            Failure::new(
                                crate::xlsx::diagnostic::Category::InternalError,
                                "numeric_lexeme_missing",
                            )
                        })?,
                        context,
                    ),
                    CellValue::Text(text) => formatter.text(text),
                    _ => unreachable!(),
                };
            }
            return Err(error);
        }
    };
    match &value.value {
        CellValue::Number(_) => {
            formatter.number(value.numeric_lexeme.as_deref().ok_or_else(|| {
                Failure::new(
                    crate::xlsx::diagnostic::Category::InternalError,
                    "numeric_lexeme_missing",
                )
            })?)
        }
        CellValue::Text(text) => formatter.text(text),
        _ => unreachable!(),
    }
}

/// General 数値だけを幅依存組版へ渡す。数式も保存済みの型と字句を使う。
pub(crate) fn general_numeric_lexeme<'a>(
    cell: &'a crate::xlsx::workbook::Cell,
    styles: &crate::xlsx::styles::Styles,
) -> Result<Option<&'a str>> {
    if !matches!(cell.value, crate::xlsx::workbook::CellValue::Number(_)) {
        return Ok(None);
    }
    let id = styles.validate_cell(cell.style)?.number_format;
    let general = styles
        .number_formats
        .get(&id)
        .map_or(id == 0, |code| code.eq_ignore_ascii_case("General"));
    if !general {
        return Ok(None);
    }
    cell.numeric_lexeme.as_deref().map(Some).ok_or_else(|| {
        Failure::new(
            crate::xlsx::diagnostic::Category::InternalError,
            "numeric_lexeme_missing",
        )
    })
}

/// 文字区分を適用しても、@ の展開元の書体境界を保持する。
/// 固定文字はセル書体、@ は各ランの書体、区分色は両方へ適用する。
pub(crate) fn rich_runs(
    runs: &[crate::xlsx::rich_text::Run],
    style: &crate::xlsx::styles::CellStyle,
    styles: &crate::xlsx::styles::Styles,
    context: FormatContext,
) -> Result<(Vec<crate::xlsx::rich_text::Run>, Option<Color>)> {
    let code = match styles.number_formats.get(&style.number_format) {
        Some(code) => code.as_str(),
        None => builtin_with_context(style.number_format, context)?,
    };
    if runs.len() > 4096 {
        return Err(Failure::input("text_fragment_limit"));
    }
    if code.eq_ignore_ascii_case("General") {
        return Ok((runs.to_vec(), None));
    }
    let formatter = match NumberFormat::parse_with_locale(code, context.locale) {
        Ok(formatter) => formatter,
        Err(error) => composite::Composite::parse(code, context)
            .map(|composite| composite.selector)
            .map_err(|_| error)?,
    };
    let Some(section) = formatter.text_section() else {
        return Ok((runs.to_vec(), None));
    };
    let mut result = Vec::new();
    for token in &section.tokens {
        match token {
            Token::At => {
                if result.len().saturating_add(runs.len()) > 4096 {
                    return Err(Failure::input("text_fragment_limit"));
                }
                result.extend_from_slice(runs);
            }
            Token::Space(_) | Token::Fill(_) => {
                return Err(Failure::unsupported("rich_text_format_spacing"));
            }
            token => {
                let mut fragments = Vec::new();
                emit(token, Some(""), &mut fragments)?;
                for fragment in fragments {
                    if let Fragment::Text(text) = fragment {
                        if result.len() == 4096 {
                            return Err(Failure::input("text_fragment_limit"));
                        }
                        result.push(crate::xlsx::rich_text::Run {
                            text: text.into(),
                            font: None,
                        });
                    }
                }
            }
        }
    }
    Ok((result, section.color.clone()))
}
