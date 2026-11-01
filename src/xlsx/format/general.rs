//! General の表示候補。採用する桁数は実フォント幅を計測して決める。
use super::decimal::Decimal;
use crate::xlsx::diagnostic::Result;

pub(crate) struct Candidate {
    pub text: String,
    pub precision: usize,
}
pub(crate) struct Candidates {
    pub fixed: Vec<Candidate>,
    pub scientific: Vec<Candidate>,
    pub order: i32,
    pub significant_digits: usize,
}

fn mantissa(integer: String, fraction: String) -> String {
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() {
        integer
    } else {
        format!("{integer}.{fraction}")
    }
}

/// ECMA-376 §18.8.30 の表示上限11文字を符号を除いて適用する。
/// 四捨五入は XML の十進字句から行い、二進浮動小数点へ往復しない。
pub(crate) fn candidates(raw: &str) -> Result<Candidates> {
    let value = Decimal::parse(raw)?;
    let order = value.order();
    let significant_digits = value.significant_digits();
    let sign = if value.negative { "-" } else { "" };
    let mut fixed = Vec::<Candidate>::new();
    let mut scientific = Vec::<Candidate>::new();
    for places in (0..=10).rev() {
        let (integer, fraction) = if order <= 10 || value.is_zero() {
            value.fixed(places, 0)?
        } else {
            (String::new(), String::new())
        };
        let text = mantissa(integer, fraction);
        // 非ゼロ値を0に丸めて収めず、指数表記で非ゼロ桁を残す。
        if !text.is_empty() && text.len() <= 11 && (value.is_zero() || text != "0") {
            let text = format!("{sign}{text}");
            if fixed.last().is_none_or(|previous| previous.text != text) {
                fixed.push(Candidate {
                    text,
                    precision: (order + places as i32 + 1).max(1) as usize,
                });
            }
        }
        if !value.is_zero() {
            let (normalized, exponent) = value.scientific(places)?;
            let (integer, fraction) = normalized.fixed(places, 0)?;
            let text = format!("{}E{:+03}", mantissa(integer, fraction), exponent);
            if text.len() <= 11 {
                let text = format!("{sign}{text}");
                if scientific
                    .last()
                    .is_none_or(|previous| previous.text != text)
                {
                    scientific.push(Candidate {
                        text,
                        precision: places + 1,
                    });
                }
            }
        }
    }
    Ok(Candidates {
        fixed,
        scientific,
        order,
        significant_digits,
    })
}
