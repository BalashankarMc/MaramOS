//! Time unit conversion helpers.
//!
//! [`Time`] enum variants represent nanoseconds through days, with
//! [`to_nanos`](Time::to_nanos) for conversion and `Mul` for scaling.

use core::ops::Mul;

#[allow(dead_code)]
pub enum Time {
    Nanoseconds(u64),
    Milliseconds(u64),
    Seconds(u64),
    Minutes(u32),
    Hours(u32),
    Days(u32),
}

impl Time {
    pub const fn to_nanos(&self) -> u64 {
        match self {
            Self::Nanoseconds(x) => *x,
            Self::Milliseconds(x) => 1_000_000 * x,
            Self::Seconds(x) => Self::Milliseconds(1000).to_nanos() * x,
            Self::Minutes(x) => Self::Seconds(60).to_nanos() * (*x as u64),
            Self::Hours(x) => Self::Minutes(60).to_nanos() * (*x as u64),
            Self::Days(x) => Self::Hours(24).to_nanos() * (*x as u64),
        }
    }
}

impl Mul<u64> for Time {
    type Output = Self;
    fn mul(self, rhs: u64) -> Self::Output {
        Self::Nanoseconds(self.to_nanos() * rhs)
    }
}