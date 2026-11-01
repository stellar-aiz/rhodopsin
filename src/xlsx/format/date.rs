use crate::xlsx::diagnostic::{Failure, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateSystem {
    Excel1900,
    Excel1904,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    English,
    Japanese,
}
impl Locale {
    /// 対応済み LCID を3〜8桁の16進表記、または既存の言語タグから解決する。
    /// 上位ビットを切り捨てない。別の暦や並べ替え指定を同じロケールに近似しない。
    pub(super) fn from_identifier(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("ja-jp") {
            return Some(Self::Japanese);
        }
        if value.eq_ignore_ascii_case("en-us") {
            return Some(Self::English);
        }
        if !(3..=8).contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        match u32::from_str_radix(value, 16).ok()? {
            0x411 => Some(Self::Japanese),
            0x409 => Some(Self::English),
            _ => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: u32,
    pub month: u32,
    pub day: u32,
    pub weekday: usize,
}

fn days_before_year(year: u32) -> u32 {
    let n = year - 1;
    365 * n + n / 4 - n / 100 + n / 400
}
fn leap(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

pub fn from_serial(day: u32, system: DateSystem) -> Result<Date> {
    let (absolute, weekday) = match system {
        DateSystem::Excel1900 => {
            if day > 2_958_465 {
                return Err(Failure::unsupported("date_out_of_range"));
            }
            let weekday = ((day + 6) % 7) as usize;
            if day == 0 {
                return Ok(Date {
                    year: 1900,
                    month: 1,
                    day: 0,
                    weekday,
                });
            }
            if day == 60 {
                return Ok(Date {
                    year: 1900,
                    month: 2,
                    day: 29,
                    weekday,
                });
            }
            (
                days_before_year(1900) + day - 1 - u32::from(day > 60),
                weekday,
            )
        }
        DateSystem::Excel1904 => {
            if day > 2_957_003 {
                return Err(Failure::unsupported("date_out_of_range"));
            }
            (days_before_year(1904) + day, ((day + 5) % 7) as usize)
        }
    };
    let (mut low, mut high) = (1900, 10000);
    while low + 1 < high {
        let middle = (low + high) / 2;
        if days_before_year(middle) <= absolute {
            low = middle;
        } else {
            high = middle;
        }
    }
    let year = low;
    let mut ordinal = absolute - days_before_year(year);
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for (index, days) in months.into_iter().enumerate() {
        if ordinal < days {
            return Ok(Date {
                year,
                month: index as u32 + 1,
                day: ordinal + 1,
                weekday,
            });
        }
        ordinal -= days;
    }
    Err(Failure::input("invalid_date_serial"))
}

#[derive(Clone, Debug)]
enum Token {
    Year(usize),
    Month(usize),
    Minute(usize),
    Day(usize),
    Hour(usize),
    Second(usize),
    Fraction(usize),
    Elapsed(char, usize),
    AmPm(bool),
    Era(usize),
    EraYear(usize),
    JapaneseWeekday(usize),
    Literal(String),
}
#[derive(Clone, Debug)]
pub struct DateFormat {
    tokens: Vec<Token>,
    locale: Locale,
    color: Option<crate::xlsx::styles::Color>,
    full_width_digits: bool,
}

impl DateFormat {
    pub fn parse(code: &str, mut locale: Locale) -> Result<Self> {
        if code.len() > 4096 {
            return Err(Failure::input("number_format_length"));
        }
        let chars: Vec<char> = code.chars().collect();
        let mut i = 0;
        let mut tokens = Vec::new();
        let mut color = None;
        let mut locale_present = false;
        let mut full_width_digits = false;
        while i < chars.len() {
            let c = chars[i];
            i += 1;
            if c == '"' {
                let mut literal = String::new();
                while i < chars.len() && chars[i] != '"' {
                    literal.push(chars[i]);
                    i += 1;
                }
                if i == chars.len() {
                    return Err(Failure::input("number_format_quote_unclosed"));
                }
                i += 1;
                tokens.push(Token::Literal(literal));
                continue;
            }
            if c == '\\' {
                let next = *chars
                    .get(i)
                    .ok_or_else(|| Failure::input("number_format_escape_missing"))?;
                i += 1;
                tokens.push(Token::Literal(next.to_string()));
                continue;
            }
            let rest: String = chars[i - 1..].iter().collect();
            if rest.len() >= 5
                && rest
                    .get(..5)
                    .is_some_and(|s| s.eq_ignore_ascii_case("AM/PM"))
            {
                i += 4;
                tokens.push(Token::AmPm(false));
                continue;
            }
            if rest.len() >= 3 && rest.get(..3).is_some_and(|s| s.eq_ignore_ascii_case("A/P")) {
                i += 2;
                tokens.push(Token::AmPm(true));
                continue;
            }
            if c == '[' {
                let mut directive = String::new();
                while i < chars.len() && chars[i] != ']' {
                    directive.push(chars[i]);
                    i += 1;
                }
                if i == chars.len() {
                    return Err(Failure::input("number_format_bracket_unclosed"));
                }
                i += 1;
                let directive = directive.to_ascii_lowercase();
                if directive == "dbnum3" {
                    if full_width_digits || !tokens.is_empty() {
                        return Err(Failure::input("date_dbnum_position"));
                    }
                    full_width_digits = true;
                    continue;
                }
                if let Some(value) = super::named_color(&directive) {
                    if color.is_some() || !tokens.is_empty() {
                        return Err(Failure::input("number_format_color_position"));
                    }
                    color = Some(value);
                    continue;
                }
                if let Some(identifier) = directive.strip_prefix("$-") {
                    if locale_present {
                        return Err(Failure::input("duplicate_number_format_locale"));
                    }
                    locale = Locale::from_identifier(identifier)
                        .ok_or_else(|| Failure::unsupported("date_format_directive"))?;
                    locale_present = true;
                    continue;
                }
                match directive.as_str() {
                    "h" | "hh" => tokens.push(Token::Elapsed('h', directive.len())),
                    "m" | "mm" => tokens.push(Token::Elapsed('m', directive.len())),
                    "s" | "ss" => tokens.push(Token::Elapsed('s', directive.len())),
                    _ => return Err(Failure::unsupported("date_format_directive")),
                }
                continue;
            }
            if c == '.'
                && matches!(
                    tokens.last(),
                    Some(Token::Second(_) | Token::Elapsed('s', _))
                )
                && chars.get(i) == Some(&'0')
            {
                let mut width = 0;
                while chars.get(i) == Some(&'0') {
                    width += 1;
                    i += 1;
                }
                if width > 3 {
                    return Err(Failure::unsupported("date_fraction_precision"));
                }
                tokens.push(Token::Fraction(width));
                continue;
            }
            let lower = c.to_ascii_lowercase();
            if matches!(lower, 'y' | 'm' | 'd' | 'h' | 's' | 'g' | 'e' | 'a') {
                let mut width = 1;
                while chars
                    .get(i)
                    .is_some_and(|c| c.to_ascii_lowercase() == lower)
                {
                    width += 1;
                    i += 1;
                }
                let token = match (lower, width) {
                    ('y', 2 | 4) => Token::Year(width),
                    ('m', 1..=5) => Token::Month(width),
                    ('d', 1..=4) => Token::Day(width),
                    ('h', 1 | 2) => Token::Hour(width),
                    ('s', 1 | 2) => Token::Second(width),
                    ('g', 1..=3) => Token::Era(width),
                    ('e', 1 | 2) => Token::EraYear(width),
                    ('a', 3 | 4) => Token::JapaneseWeekday(width),
                    _ => return Err(Failure::unsupported("date_token_width")),
                };
                tokens.push(token);
                continue;
            }
            if c.is_ascii_alphanumeric()
                || matches!(c, ';' | '@' | '#' | '?' | '_' | '*' | '%' | '[' | ']')
            {
                return Err(Failure::unsupported("date_format_token"));
            }
            tokens.push(Token::Literal(c.to_string()));
        }
        // 月/分の文脈判定では区切り文字を飛ばし、隣接する日付時刻トークンを見る。
        for i in 0..tokens.len() {
            if let Token::Month(width @ (1 | 2)) = tokens[i] {
                let previous = tokens[..i]
                    .iter()
                    .rev()
                    .find(|t| !matches!(t, Token::Literal(_)));
                let next = tokens[i + 1..]
                    .iter()
                    .find(|t| !matches!(t, Token::Literal(_)));
                if matches!(previous, Some(Token::Hour(_) | Token::Elapsed('h', _)))
                    || matches!(next, Some(Token::Second(_) | Token::Elapsed('s', _)))
                {
                    tokens[i] = Token::Minute(width);
                }
            }
        }
        if tokens.iter().all(|t| matches!(t, Token::Literal(_))) {
            return Err(Failure::unsupported("date_tokens_missing"));
        }
        if full_width_digits && locale != Locale::Japanese {
            return Err(Failure::unsupported("date_dbnum_locale"));
        }
        Ok(Self {
            tokens,
            locale,
            color,
            full_width_digits,
        })
    }

    pub fn color(&self) -> Option<&crate::xlsx::styles::Color> {
        self.color.as_ref()
    }

    pub fn format(&self, raw: &str, system: DateSystem) -> Result<String> {
        let serial = super::decimal::Decimal::parse(raw)?;
        if serial.negative {
            return Err(Failure::unsupported("negative_date_serial"));
        }
        let maximum = match system {
            DateSystem::Excel1900 => "2958466",
            DateSystem::Excel1904 => "2957004",
        };
        if !serial
            .compare(&super::decimal::Decimal::parse(maximum)?)
            .is_lt()
        {
            return Err(Failure::unsupported("date_out_of_range"));
        }
        let has_time = self.tokens.iter().any(|t| {
            matches!(
                t,
                Token::Hour(_)
                    | Token::Minute(_)
                    | Token::Second(_)
                    | Token::Elapsed(_, _)
                    | Token::AmPm(_)
            )
        });
        let precision = self
            .tokens
            .iter()
            .filter_map(|t| {
                if let Token::Fraction(n) = t {
                    Some(*n)
                } else {
                    None
                }
            })
            .max()
            .unwrap_or(0);
        let factor = 10u64.pow(precision as u32);
        let day_units = 86400 * factor;
        let (whole_day, time_units) = serial.day_parts(day_units as u32)?;
        let day = whole_day
            + if has_time {
                (time_units / day_units) as u32
            } else {
                0
            };
        let date = from_serial(day, system)?;
        let units = time_units % day_units;
        let seconds = units / factor;
        let (hour, minute, second) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
        let am_pm = self.tokens.iter().any(|t| matches!(t, Token::AmPm(_)));
        let era = if self
            .tokens
            .iter()
            .any(|t| matches!(t, Token::Era(_) | Token::EraYear(_)))
        {
            Some(era(date)?)
        } else {
            None
        };
        let mut out = String::new();
        let pad = |value: u64, width: usize| format!("{value:0width$}");
        for token in &self.tokens {
            let part = match token {
                Token::Literal(s) => s.clone(),
                Token::Year(n) => pad(
                    u64::from(if *n == 2 { date.year % 100 } else { date.year }),
                    *n,
                ),
                Token::Month(n) => month(date.month, *n, self.locale),
                Token::Minute(n) => pad(minute, *n),
                Token::Day(n @ (1 | 2)) => pad(u64::from(date.day), *n),
                Token::Day(n) => weekday(date.weekday, *n == 4, Locale::English),
                Token::JapaneseWeekday(n) => weekday(date.weekday, *n == 4, Locale::Japanese),
                Token::Hour(n) => pad(
                    if am_pm {
                        if hour % 12 == 0 { 12 } else { hour % 12 }
                    } else {
                        hour
                    },
                    *n,
                ),
                Token::Second(n) => pad(second, *n),
                Token::Fraction(n) => format!(
                    ".{}",
                    pad(units % factor / 10u64.pow((precision - n) as u32), *n)
                ),
                Token::Elapsed(unit, n) => {
                    let total = u64::from(whole_day) * 86400 + time_units / factor;
                    pad(
                        match unit {
                            'h' => total / 3600,
                            'm' => total / 60,
                            _ => total,
                        },
                        *n,
                    )
                }
                Token::AmPm(short) => if hour < 12 {
                    if *short { "A" } else { "AM" }
                } else if *short {
                    "P"
                } else {
                    "PM"
                }
                .into(),
                Token::Era(n) => {
                    let e = era.ok_or_else(|| Failure::unsupported("japanese_era_missing"))?;
                    match n {
                        1 => e.0,
                        2 => e.1,
                        _ => e.2,
                    }
                    .into()
                }
                Token::EraYear(n) => pad(
                    u64::from(
                        date.year
                            - era
                                .ok_or_else(|| Failure::unsupported("japanese_era_missing"))?
                                .3
                            + 1,
                    ),
                    *n,
                ),
            };
            // 引用・エスケープした固定文字は変換せず、生成した数字だけを変換する。
            // DBNum1/2 の月日には単位規則が必要なため、この桁置換を流用しない。
            if self.full_width_digits && !matches!(token, Token::Literal(_)) {
                const DIGITS: [char; 10] =
                    ['０', '１', '２', '３', '４', '５', '６', '７', '８', '９'];
                out.extend(part.chars().map(|ch| {
                    if ch.is_ascii_digit() {
                        DIGITS[ch as usize - '0' as usize]
                    } else {
                        ch
                    }
                }));
            } else {
                out.push_str(&part);
            }
        }
        Ok(out)
    }
}

fn month(month: u32, width: usize, locale: Locale) -> String {
    const SHORT: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const LONG: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    match width {
        1 | 2 => format!("{month:0width$}"),
        _ if locale == Locale::Japanese => format!("{month}月"),
        3 => SHORT[(month - 1) as usize].into(),
        4 => LONG[(month - 1) as usize].into(),
        _ => LONG[(month - 1) as usize][..1].into(),
    }
}
fn weekday(day: usize, long: bool, locale: Locale) -> String {
    match locale {
        Locale::English => if long {
            [
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ][day]
        } else {
            ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][day]
        }
        .into(),
        Locale::Japanese => format!(
            "{}{}",
            ["日", "月", "火", "水", "木", "金", "土"][day],
            if long { "曜日" } else { "" }
        ),
    }
}
fn era(date: Date) -> Result<(&'static str, &'static str, &'static str, u32)> {
    let key = (date.year, date.month, date.day);
    for (start, initial, short, long) in [
        ((2019, 5, 1), "R", "令", "令和"),
        ((1989, 1, 8), "H", "平", "平成"),
        ((1926, 12, 25), "S", "昭", "昭和"),
        ((1912, 7, 30), "T", "大", "大正"),
        ((1868, 10, 23), "M", "明", "明治"),
    ] {
        if key >= start {
            return Ok((initial, short, long, start.0));
        }
    }
    Err(Failure::unsupported("japanese_era_out_of_range"))
}
