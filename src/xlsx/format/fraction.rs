//! 分母を二桁までに制限した帯分数。連分数の計算は十進桁の整数演算で行う。
use super::{Fragment, Token, decimal::Decimal, push_text};
use crate::xlsx::diagnostic::{Failure, Result};

// 桁数は入力十進字句の上限に従う。因数は候補分母の99以下。
fn multiply(digits: &[u8], factor: u32) -> Vec<u8> {
    let mut out = vec![0; digits.len()];
    let mut carry = 0;
    for (i, digit) in digits.iter().enumerate().rev() {
        let n = u32::from(*digit) * factor + carry;
        out[i] = (n % 10) as u8;
        carry = n / 10;
    }
    while carry != 0 {
        out.insert(0, (carry % 10) as u8);
        carry /= 10;
    }
    trim(&out).to_vec()
}
fn trim(digits: &[u8]) -> &[u8] {
    &digits[digits.iter().position(|d| *d != 0).unwrap_or(digits.len())..]
}
fn compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    let (a, b) = (trim(a), trim(b));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}
fn subtract(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = a.to_vec();
    let mut borrow = 0i16;
    for i in (0..out.len()).rev() {
        let other = b.len().checked_sub(out.len() - i).map_or(0, |j| b[j]);
        let n = i16::from(out[i]) - i16::from(other) - borrow;
        borrow = i16::from(n < 0);
        out[i] = n.rem_euclid(10) as u8;
    }
    trim(&out).to_vec()
}
fn convergent(fraction: &[u8], limit: u32) -> (u32, u32) {
    if fraction.is_empty() {
        return (0, 1);
    }
    let mut a = vec![0; fraction.len() + 1];
    a[0] = 1;
    let mut b = trim(fraction).to_vec();
    let (mut old_n, mut n, mut old_d, mut d) = (1, 0, 0, 1);
    while !b.is_empty() {
        let cap = (limit - old_d) / d;
        // この段の分母が上限を超えるなら、直前の収束分数を使う。
        if !compare(&a, &multiply(&b, cap + 1)).is_lt() {
            break;
        }
        let (mut low, mut high) = (0, cap + 1);
        while low + 1 < high {
            let middle = (low + high) / 2;
            if compare(&multiply(&b, middle), &a).is_gt() {
                high = middle;
            } else {
                low = middle;
            }
        }
        let remainder = subtract(&a, &multiply(&b, low));
        (old_n, n) = (n, old_n + low * n);
        (old_d, d) = (d, old_d + low * d);
        a = b;
        b = remainder;
    }
    (n, d)
}
fn increment(whole: &mut String) {
    let mut digits = whole.as_bytes().to_vec();
    for digit in digits.iter_mut().rev() {
        if *digit < b'9' {
            *digit += 1;
            *whole = String::from_utf8(digits).unwrap();
            return;
        }
        *digit = b'0';
    }
    digits.insert(0, b'1');
    *whole = String::from_utf8(digits).unwrap();
}

pub(super) fn render(
    value: &Decimal,
    tokens: &[Token],
    shift: i32,
    out: &mut Vec<Fragment>,
) -> Result<()> {
    let invalid = || Failure::unsupported("fraction_number_pattern");
    // 組み込み12/13と同じ帯分数。固定分母、仮分数、百分率・桁区切りは別対応。
    if shift != 0
        || !matches!(tokens.first(), Some(Token::Digit('#' | '0')))
        || !matches!(tokens.get(1), Some(Token::Text(space)) if space == " ")
    {
        return Err(invalid());
    }
    let slash = tokens
        .iter()
        .position(|token| matches!(token, Token::Slash))
        .ok_or_else(invalid)?;
    let numerator = tokens.get(2..slash).ok_or_else(invalid)?;
    let denominator = &tokens[slash + 1..];
    if !(1..=2).contains(&numerator.len())
        || !(1..=2).contains(&denominator.len())
        || numerator
            .iter()
            .chain(denominator)
            .any(|token| !matches!(token, Token::Digit('?')))
    {
        return Err(invalid());
    }
    let (mut whole, decimals) = value.fraction_parts()?;
    let (mut n, d) = convergent(&decimals, if denominator.len() == 1 { 9 } else { 99 });
    if n == d {
        increment(&mut whole);
        n = 0;
    }
    if whole != "0" || n == 0 || matches!(tokens[0], Token::Digit('0')) {
        push_text(out, &whole);
    }
    push_text(out, " ");
    if n == 0 {
        out.extend((0..numerator.len()).map(|_| Fragment::Space('0')));
        out.push(Fragment::Space('/'));
        out.extend((0..denominator.len()).map(|_| Fragment::Space('0')));
    } else {
        let n = n.to_string();
        let d = d.to_string();
        out.extend((n.len()..numerator.len()).map(|_| Fragment::Space('0')));
        push_text(out, &n);
        push_text(out, "/");
        // 分母は左詰めし、末尾の不足桁を空白幅で確保する。
        push_text(out, &d);
        out.extend((d.len()..denominator.len()).map(|_| Fragment::Space('0')));
    }
    Ok(())
}
