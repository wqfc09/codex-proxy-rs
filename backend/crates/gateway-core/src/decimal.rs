//! 账户策略与用量共同使用的非负定点数；不依赖执行、路由或计价规则。

use std::{fmt, str::FromStr};

use crate::validation::MeteringError;

const DECIMAL_SCALE: u128 = 10_000_000_000;
const MAX_SCALED_DECIMAL: u128 = 99_999_999_999_999_999_999;

/// 与 PostgreSQL `numeric(20, 10)` 对齐的非负十进制定点值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Decimal(u128);

impl Decimal {
    pub const ZERO: Self = Self(0);
    /// 数据库可表示的最大非负金额。
    pub const MAX: Self = Self(MAX_SCALED_DECIMAL);

    /// 从按十位小数缩放的整数创建。
    ///
    /// # Errors
    ///
    /// 超出数据库范围时返回错误。
    pub const fn from_scaled(value: u128) -> Result<Self, MeteringError> {
        if value > MAX_SCALED_DECIMAL {
            return Err(MeteringError::InvalidDecimal);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn scaled(self) -> u128 {
        self.0
    }

    /// 非负金额相减，超支时返回零。
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    #[must_use]
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0
            .checked_add(other.0)
            .filter(|value| *value <= MAX_SCALED_DECIMAL)
            .map(Self)
    }

    /// 以数据库定点刻度相乘，并截断到最多十位小数。
    #[must_use]
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        self.0
            .checked_mul(other.0)
            .map(|value| value / DECIMAL_SCALE)
            .filter(|value| *value <= MAX_SCALED_DECIMAL)
            .map(Self)
    }

    /// 除以非零整数，保留最多十位小数。
    #[must_use]
    pub fn checked_div_u64(self, divisor: u64) -> Option<Self> {
        let divisor = u128::from(divisor);
        (divisor != 0)
            .then(|| self.0.checked_div(divisor))
            .flatten()
            .and_then(|value| Self::from_scaled(value).ok())
    }

    /// 去尾零的 canonical 字符串，用于 wire 序列化。
    #[must_use]
    pub fn canonical(self) -> String {
        let integer = self.0 / DECIMAL_SCALE;
        let fraction = self.0 % DECIMAL_SCALE;
        if fraction == 0 {
            integer.to_string()
        } else {
            let fraction = format!("{fraction:010}");
            let trimmed = fraction.trim_end_matches('0');
            format!("{integer}.{trimmed}")
        }
    }
}

impl FromStr for Decimal {
    type Err = MeteringError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() || value.starts_with(['-', '+']) {
            return Err(MeteringError::InvalidDecimal);
        }
        let mut parts = value.split('.');
        let integer = parts.next().ok_or(MeteringError::InvalidDecimal)?;
        let fraction = parts.next().unwrap_or("");
        if parts.next().is_some()
            || integer.is_empty()
            || integer.len() > 10
            || !integer.bytes().all(|byte| byte.is_ascii_digit())
            || fraction.len() > 10
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(MeteringError::InvalidDecimal);
        }

        let integer = integer
            .parse::<u128>()
            .map_err(|_| MeteringError::InvalidDecimal)?;
        let fraction = if fraction.is_empty() {
            0
        } else {
            fraction
                .parse::<u128>()
                .map_err(|_| MeteringError::InvalidDecimal)?
                * 10_u128.pow(10_u32.saturating_sub(fraction.len() as u32))
        };
        let scaled = integer
            .checked_mul(DECIMAL_SCALE)
            .and_then(|whole| whole.checked_add(fraction))
            .ok_or(MeteringError::InvalidDecimal)?;
        Self::from_scaled(scaled)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let integer = self.0 / DECIMAL_SCALE;
        let fraction = self.0 % DECIMAL_SCALE;
        write!(formatter, "{integer}.{fraction:010}")
    }
}
