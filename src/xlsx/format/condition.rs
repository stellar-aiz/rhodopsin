use super::decimal::Decimal;
use crate::xlsx::diagnostic::{Failure, Result};
use std::cmp::Ordering;

#[derive(Clone, Debug)]
pub(super) struct Condition {
    operator: Operator,
    value: Decimal,
}
#[derive(Clone, Debug)]
enum Operator {
    Less,
    LessEqual,
    Equal,
    NotEqual,
    GreaterEqual,
    Greater,
}
impl Condition {
    pub fn parse(code: &str) -> Result<Self> {
        let (prefix, operator) = [
            ("<=", Operator::LessEqual),
            ("<>", Operator::NotEqual),
            (">=", Operator::GreaterEqual),
            ("<", Operator::Less),
            ("=", Operator::Equal),
            (">", Operator::Greater),
        ]
        .into_iter()
        .find(|(prefix, _)| code.starts_with(prefix))
        .ok_or_else(|| Failure::input("invalid_number_format_condition"))?;
        let value = Decimal::parse(&code[prefix.len()..])
            .map_err(|_| Failure::input("invalid_number_format_condition"))?;
        Ok(Self { operator, value })
    }
    pub fn matches(&self, value: &Decimal) -> bool {
        let comparison = value.compare(&self.value);
        match self.operator {
            Operator::Less => comparison == Ordering::Less,
            Operator::LessEqual => comparison != Ordering::Greater,
            Operator::Equal => comparison == Ordering::Equal,
            Operator::NotEqual => comparison != Ordering::Equal,
            Operator::GreaterEqual => comparison != Ordering::Less,
            Operator::Greater => comparison == Ordering::Greater,
        }
    }
}
