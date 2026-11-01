use crate::xlsx::diagnostic::{Failure, Result};

/// XML の十進表記を保持し、二進浮動小数点への往復で表示丸めを変えない。
#[derive(Clone, Debug)]
pub struct Decimal {
    pub negative: bool,
    digits: Vec<u8>,
    exponent: i32,
}
impl Decimal {
    pub fn parse(raw: &str) -> Result<Self> {
        let bad = || Failure::input("invalid_numeric_value");
        if raw.is_empty() || raw.len() > 1024 {
            return Err(bad());
        }
        let (negative, raw) = if let Some(s) = raw.strip_prefix('-') {
            (true, s)
        } else {
            (false, raw.strip_prefix('+').unwrap_or(raw))
        };
        let (mantissa, exponent) = match raw.find(['e', 'E']) {
            Some(i) => (&raw[..i], raw[i + 1..].parse::<i32>().map_err(|_| bad())?),
            None => (raw, 0),
        };
        if !(-1024..=1024).contains(&exponent) {
            return Err(bad());
        }
        let mut point = false;
        let mut fractional = 0;
        let mut digits = Vec::new();
        for b in mantissa.bytes() {
            match b {
                b'.' if !point => point = true,
                b'0'..=b'9' => {
                    digits.push(b - b'0');
                    if point {
                        fractional += 1;
                    }
                }
                _ => return Err(bad()),
            }
        }
        if digits.is_empty() {
            return Err(bad());
        }
        let first = digits
            .iter()
            .position(|d| *d != 0)
            .unwrap_or(digits.len() - 1);
        digits.drain(..first);
        Ok(Self {
            negative: negative && digits != [0],
            digits,
            exponent: exponent - fractional,
        })
    }

    pub(super) fn order(&self) -> i32 {
        self.digits.len() as i32 + self.exponent - 1
    }

    pub(super) fn significant_digits(&self) -> usize {
        self.digits
            .iter()
            .rposition(|digit| *digit != 0)
            .map_or(1, |i| i + 1)
    }

    /// 絶対値を整数部と、丸めていない小数の十進桁へ分ける。
    pub(super) fn fraction_parts(&self) -> Result<(String, Vec<u8>)> {
        if self.exponent >= 0 {
            return Ok((self.fixed(0, 0)?.0, Vec::new()));
        }
        let split = self.digits.len() as i32 + self.exponent;
        let whole = if split > 0 {
            self.digits[..split as usize]
                .iter()
                .map(|d| char::from(b'0' + d))
                .collect()
        } else {
            "0".into()
        };
        let mut fraction = vec![0; (-split).max(0) as usize];
        fraction.extend_from_slice(&self.digits[split.max(0) as usize..]);
        while fraction.last() == Some(&0) {
            fraction.pop();
        }
        Ok((whole, fraction))
    }

    pub fn is_zero(&self) -> bool {
        self.digits.iter().all(|d| *d == 0)
    }

    /// 非負値の整数部と、小数部を scale 倍して四捨五入した整数。
    /// 日付の通算日と一日内の時刻を、二進浮動小数点を介さず分離する。
    pub fn day_parts(&self, scale: u32) -> Result<(u32, u64)> {
        let bad = || Failure::unsupported("date_out_of_range");
        if self.negative {
            return Err(Failure::unsupported("negative_date_serial"));
        }
        if self.is_zero() {
            return Ok((0, 0));
        }
        let split = (self.digits.len() as i32 + self.exponent).max(0) as usize;
        if split > 10 {
            return Err(bad());
        }
        let mut whole = 0u32;
        for index in 0..split {
            whole = whole
                .checked_mul(10)
                .and_then(|value| {
                    value.checked_add(u32::from(*self.digits.get(index).unwrap_or(&0)))
                })
                .ok_or_else(bad)?;
        }
        if split >= self.digits.len() {
            return Ok((whole, 0));
        }
        let mut fraction = Self {
            negative: false,
            digits: self.digits[split..].to_vec(),
            exponent: self.exponent,
        };
        let mut carry = 0u64;
        for digit in fraction.digits.iter_mut().rev() {
            let value = u64::from(*digit) * u64::from(scale) + carry;
            *digit = (value % 10) as u8;
            carry = value / 10;
        }
        let mut prefix = Vec::new();
        while carry != 0 {
            prefix.push((carry % 10) as u8);
            carry /= 10;
        }
        prefix.reverse();
        prefix.append(&mut fraction.digits);
        fraction.digits = prefix;
        let units = fraction.fixed(0, 0)?.0.parse::<u64>().map_err(|_| bad())?;
        Ok((whole, units))
    }

    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        if self.is_zero() && other.is_zero() {
            return Ordering::Equal;
        }
        if self.negative != other.negative {
            return if self.negative {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        let magnitude = if self.is_zero() {
            Ordering::Less
        } else if other.is_zero() {
            Ordering::Greater
        } else {
            (self.digits.len() as i32 + self.exponent)
                .cmp(&(other.digits.len() as i32 + other.exponent))
                .then_with(|| {
                    (0..self.digits.len().max(other.digits.len()))
                        .map(|i| {
                            self.digits
                                .get(i)
                                .unwrap_or(&0)
                                .cmp(other.digits.get(i).unwrap_or(&0))
                        })
                        .find(|ordering| *ordering != Ordering::Equal)
                        .unwrap_or(Ordering::Equal)
                })
        };
        if self.negative {
            magnitude.reverse()
        } else {
            magnitude
        }
    }

    /// 一桁の仮数へ正規化する。丸めで 10 になる場合は指数も繰り上げる。
    pub fn scientific(&self, places: usize) -> Result<(Self, i32)> {
        self.scientific_group(places, 1)
    }

    /// 指数を group の倍数にし、仮数を 1 以上 10^group 未満にする。
    /// 負の指数もゼロ方向ではなく下方へ揃える。丸めの桁上がりは群単位。
    pub fn scientific_group(&self, places: usize, group: i32) -> Result<(Self, i32)> {
        if !(1..=3).contains(&group) {
            return Err(Failure::unsupported("scientific_number_pattern"));
        }
        if self.is_zero() {
            return Ok((
                Self {
                    negative: false,
                    digits: vec![0],
                    exponent: 0,
                },
                0,
            ));
        }
        let order = self.digits.len() as i32 + self.exponent - 1;
        let mut exponent = order.div_euclid(group) * group;
        let mut mantissa = Self {
            negative: false,
            digits: self.digits.clone(),
            exponent: self.exponent - exponent,
        };
        if mantissa.fixed(places, 0)?.0.len() > group as usize {
            mantissa.exponent -= group;
            exponent += group;
        }
        Ok((mantissa, exponent))
    }

    /// 小数点以下 places 桁へ四捨五入した絶対値を返す。
    pub fn fixed(&self, places: usize, shift: i32) -> Result<(String, String)> {
        if places > 30 || !(-1024..=1024).contains(&shift) {
            return Err(Failure::unsupported("number_format_precision"));
        }
        let keep = self.digits.len() as i32 + self.exponent + shift + places as i32;
        if keep > 2048 {
            return Err(Failure::input("formatted_number_limit"));
        }
        let mut output = if keep <= 0 {
            vec![u8::from(keep == 0 && self.digits[0] >= 5)]
        } else {
            let keep = keep as usize;
            let mut out = self.digits[..keep.min(self.digits.len())].to_vec();
            out.resize(keep, 0);
            if self.digits.get(keep).is_some_and(|d| *d >= 5) {
                let mut carry = true;
                for digit in out.iter_mut().rev() {
                    if *digit == 9 {
                        *digit = 0;
                    } else {
                        *digit += 1;
                        carry = false;
                        break;
                    }
                }
                if carry {
                    out.insert(0, 1);
                }
            }
            out
        };
        if output.len() <= places {
            let mut padding = vec![0; places + 1 - output.len()];
            padding.append(&mut output);
            output = padding;
        }
        let split = output.len() - places;
        let text = |ds: &[u8]| ds.iter().map(|d| char::from(b'0' + d)).collect::<String>();
        Ok((text(&output[..split]), text(&output[split..])))
    }
}
