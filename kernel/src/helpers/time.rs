//! Time unit conversion helpers.

use core::ops::Mul;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum Time {
    Nanoseconds(u64),
    Milliseconds(u64),
    Seconds(u64),
    Minutes(u32),
    Hours(u32),
    Days(u32),
}

impl Time {
    pub fn to_nanos(&self) -> u64 {
        match self {
            Self::Nanoseconds(x) => *x,
            Self::Milliseconds(x) => 1_000_000 * x,
            Self::Seconds(x) => Self::Milliseconds(1000).to_nanos() * x,
            Self::Minutes(x) => Self::Seconds(60).to_nanos() * (u64::from(*x)),
            Self::Hours(x) => Self::Minutes(60).to_nanos() * (u64::from(*x)),
            Self::Days(x) => Self::Hours(24).to_nanos() * (u64::from(*x)),
        }
    }
}

impl Mul<u64> for Time {
    type Output = Self;
    fn mul(self, rhs: u64) -> Self::Output {
        Self::Nanoseconds(self.to_nanos() * rhs)
    }
}