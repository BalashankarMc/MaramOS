//! Time unit conversion helpers.

use core::ops::Mul;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Time {
    Nanoseconds(u64),
    Milliseconds(u64),
    Seconds(u64),
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

impl Time {
    pub const fn to_nanos(self) -> u64 {
        match self {
            Self::Nanoseconds(x) => x,
            Self::Milliseconds(x) => x * 1_000_000,
            Self::Seconds(x) => x * 1_000_000_000,
            Self::Minutes(x) => x * 60_000_000_000,
            Self::Hours(x) => x * 3_600_000_000_000,
            Self::Days(x) => x * 86_400_000_000_000,
        }
    }
}

impl Mul<u64> for Time {
    type Output = Self;
    fn mul(self, rhs: u64) -> Self::Output {
        Self::Nanoseconds(self.to_nanos() * rhs)
    }
}