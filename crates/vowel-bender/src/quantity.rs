//! Types for quantities, so the compiler checks units and ranges instead of doc comments describing them.
//!
//! Each physical quantity stores its value in one unit and has constructors and accessors that name the unit,
//! such as [`Frequency::from_hertz`] and [`Frequency::hertz`].
//! Adding or subtracting two values of the same quantity, and scaling one by a plain number, give the same quantity.
//! Dividing two values of the same quantity gives a plain ratio.
//!
//! Dimensionless values with a limited range, such as gains and fractions, get types that check the range.
//! Audio signals stay plain `f64` buffers, nominally from −1 to 1, so filter equations read as arithmetic.

use std::fmt;
use std::iter::Product;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

use serde::{Deserialize, Serialize};

/// Implements arithmetic that keeps the quantity, for a type with one `f64` field.
macro_rules! linear_quantity {
    ($name:ident, $field:ident) => {
        impl $name {
            pub const ZERO: Self = Self { $field: 0.0 };

            #[must_use]
            pub fn abs(self) -> Self {
                Self {
                    $field: self.$field.abs(),
                }
            }
        }

        impl Add for $name {
            type Output = Self;

            fn add(self, other: Self) -> Self {
                Self {
                    $field: self.$field + other.$field,
                }
            }
        }

        impl AddAssign for $name {
            fn add_assign(&mut self, other: Self) {
                self.$field += other.$field;
            }
        }

        impl Sub for $name {
            type Output = Self;

            fn sub(self, other: Self) -> Self {
                Self {
                    $field: self.$field - other.$field,
                }
            }
        }

        impl SubAssign for $name {
            fn sub_assign(&mut self, other: Self) {
                self.$field -= other.$field;
            }
        }

        impl Neg for $name {
            type Output = Self;

            fn neg(self) -> Self {
                Self {
                    $field: -self.$field,
                }
            }
        }

        impl Mul<f64> for $name {
            type Output = Self;

            fn mul(self, factor: f64) -> Self {
                Self {
                    $field: self.$field * factor,
                }
            }
        }

        impl Mul<$name> for f64 {
            type Output = $name;

            fn mul(self, quantity: $name) -> $name {
                quantity * self
            }
        }

        impl Div<f64> for $name {
            type Output = Self;

            fn div(self, divisor: f64) -> Self {
                Self {
                    $field: self.$field / divisor,
                }
            }
        }

        /// The ratio of two values.
        impl Div for $name {
            type Output = f64;

            fn div(self, divisor: Self) -> f64 {
                self.$field / divisor.$field
            }
        }
    };
}

/// A frequency, such as a formant, a bandwidth or a fundamental frequency.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Frequency {
    hertz: f64,
}

linear_quantity!(Frequency, hertz);

impl Frequency {
    #[must_use]
    pub const fn from_hertz(hertz: f64) -> Self {
        Self { hertz }
    }

    #[must_use]
    pub const fn hertz(self) -> f64 {
        self.hertz
    }
}

/// Writes the value in hertz, honouring precision such as `{:.1}`.
impl fmt::Display for Frequency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.hertz, formatter)?;

        formatter.write_str(" Hz")
    }
}

/// A point in time or a duration.
/// It can be negative, for example as a difference between two times.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Time {
    seconds: f64,
}

linear_quantity!(Time, seconds);

impl Time {
    #[must_use]
    pub const fn from_seconds(seconds: f64) -> Self {
        Self { seconds }
    }

    #[must_use]
    pub const fn from_milliseconds(milliseconds: f64) -> Self {
        Self {
            seconds: milliseconds / 1000.0,
        }
    }

    #[must_use]
    pub const fn seconds(self) -> f64 {
        self.seconds
    }

    #[must_use]
    pub const fn milliseconds(self) -> f64 {
        self.seconds * 1000.0
    }
}

/// Writes the value in seconds, honouring precision such as `{:.3}`.
impl fmt::Display for Time {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.seconds, formatter)?;

        formatter.write_str(" s")
    }
}

/// Cycles completed in a duration.
impl Mul<Time> for Frequency {
    type Output = f64;

    fn mul(self, duration: Time) -> f64 {
        self.hertz * duration.seconds
    }
}

/// Cycles completed in a duration.
impl Mul<Frequency> for Time {
    type Output = f64;

    fn mul(self, frequency: Frequency) -> f64 {
        frequency * self
    }
}

/// A pitch interval; twelve semitones make an octave.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Interval {
    semitones: f64,
}

linear_quantity!(Interval, semitones);

impl Interval {
    #[must_use]
    pub const fn from_semitones(semitones: f64) -> Self {
        Self { semitones }
    }

    #[must_use]
    pub const fn semitones(self) -> f64 {
        self.semitones
    }

    /// The frequency ratio of this interval, for example 2 for twelve semitones.
    #[must_use]
    pub fn frequency_ratio(self) -> f64 {
        let octaves = self.semitones / 12.0;

        f64::powf(2.0, octaves)
    }
}

/// Moves a frequency by an interval.
impl Mul<Interval> for Frequency {
    type Output = Frequency;

    fn mul(self, interval: Interval) -> Frequency {
        self * interval.frequency_ratio()
    }
}

/// Samples per second of a digital signal.
/// It is an integer, as in WAV files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SampleRate {
    hertz: u32,
}

impl SampleRate {
    #[must_use]
    pub const fn from_hertz(hertz: u32) -> Self {
        Self { hertz }
    }

    #[must_use]
    pub const fn hertz(self) -> u32 {
        self.hertz
    }

    #[must_use]
    pub fn frequency(self) -> Frequency {
        Frequency::from_hertz(f64::from(self.hertz))
    }

    /// The highest frequency the signal can represent: half the sample rate.
    #[must_use]
    pub fn nyquist_frequency(self) -> Frequency {
        self.frequency() / 2.0
    }

    /// The number of whole samples in a duration, rounded to the nearest sample.
    ///
    /// # Panics
    ///
    /// Panics when the duration is negative.
    #[must_use]
    pub fn sample_count(self, duration: Time) -> usize {
        assert!(duration >= Time::ZERO, "negative duration: {duration}");

        let sample_count = (duration * self.frequency()).round();

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the count is non-negative, and a count above usize::MAX saturates"
        )]
        let sample_count = sample_count as usize;

        sample_count
    }

    /// The time of the sample at `index`, counted from the first sample.
    #[must_use]
    pub fn time_of_sample(self, index: usize) -> Time {
        #[expect(
            clippy::cast_precision_loss,
            reason = "indices of audio samples stay far below 2^52"
        )]
        let index = index as f64;

        Time::from_seconds(index / f64::from(self.hertz))
    }
}

impl fmt::Display for SampleRate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} Hz", self.hertz)
    }
}

/// A linear amplitude gain: 1 leaves a signal unchanged, 0 silences it, and it is never negative.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Gain {
    linear: f64,
}

impl Gain {
    pub const SILENCE: Self = Self { linear: 0.0 };
    pub const UNITY: Self = Self { linear: 1.0 };

    /// # Panics
    ///
    /// Panics when `linear` is negative or not a number.
    #[must_use]
    pub const fn from_linear(linear: f64) -> Self {
        assert!(linear >= 0.0, "a gain cannot be negative or NaN");

        Self { linear }
    }

    #[must_use]
    pub const fn linear(self) -> f64 {
        self.linear
    }

    /// The gain in decibels: 0 for unity, about +6 for doubling, and minus infinity for silence.
    #[must_use]
    pub fn decibels(self) -> f64 {
        20.0 * self.linear.log10()
    }
}

/// Gains in series multiply.
impl Mul for Gain {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self {
            linear: self.linear * other.linear,
        }
    }
}

impl Product for Gain {
    fn product<I>(gains: I) -> Self
    where
        I: Iterator<Item = Self>,
    {
        gains.fold(Gain::UNITY, Mul::mul)
    }
}

/// Applies a gain to a signal sample.
impl Mul<Gain> for f64 {
    type Output = f64;

    fn mul(self, gain: Gain) -> f64 {
        self * gain.linear
    }
}

impl fmt::Display for Gain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.linear, formatter)?;

        formatter.write_str("×")
    }
}

/// A share of a whole, from 0 to 1, such as a phase within a period or a level relative to full scale.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct Fraction {
    value: f64,
}

impl Fraction {
    pub const ZERO: Self = Self { value: 0.0 };
    pub const ONE: Self = Self { value: 1.0 };

    /// # Panics
    ///
    /// Panics when `value` is outside 0 to 1 or not a number.
    #[must_use]
    pub const fn new(value: f64) -> Self {
        assert!(
            value >= 0.0 && value <= 1.0,
            "a fraction must be from 0 to 1"
        );

        Self { value }
    }

    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }
}

impl fmt::Display for Fraction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, formatter)
    }
}
