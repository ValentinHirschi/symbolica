use std::{
    fmt::{self, Debug, Display, Formatter, LowerExp, Write},
    ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Sub, SubAssign},
};

use rand::Rng;

use super::{FixedPrecision, Float, FloatLike, Real, RealLike, SingleFloat};
use crate::domains::{integer::Integer, rational::Rational};

/// A float that does linear error propagation.
///
/// The absolute error is stored as an `f64`, independently of the value type.
/// `new`, `new_with_accuracy`, `From<f64>`, multiplication, division, and inversion
/// return a NaN error
/// when a nonzero propagated error cannot be represented in that range. This
/// leaves the native value arithmetic intact; it does not certify an enclosure.
/// Fallback calculations retain the native domain's intermediate exponent limits.
/// Check [`Self::get_absolute_error`] separately from the value's finiteness.
#[derive(Copy, Clone)]
pub struct ErrorPropagatingFloat<T: FloatLike> {
    value: T,
    abs_err: f64,
}

// A zero from a positive mathematical error is a range failure, not exactness.
fn checked_error(error: f64, nonzero: bool) -> f64 {
    if error.is_finite() && error >= 0. && (!nonzero || error > 0.) {
        error
    } else {
        f64::NAN
    }
}

fn accuracy_error(accuracy: f64) -> f64 {
    checked_error(10f64.powf(-accuracy), accuracy != f64::INFINITY)
}

fn magnitude<T: RealLike>(value: &T) -> T {
    if value.to_f64().is_sign_negative() {
        -value.clone()
    } else {
        value.clone()
    }
}

// Retain the completed sum in the native domain before narrowing the error.
fn native_error<T: RealLike>(a: &T, ea: f64, b: &T, eb: f64, divide: bool) -> f64 {
    let da = a.from_rational(&Rational::try_from(ea).unwrap());
    let db = b.from_rational(&Rational::try_from(eb).unwrap());
    let a = magnitude(a);
    let b = magnitude(b);
    let result = if divide {
        da / &b + (db / &b) * (a / &b)
    } else if a.is_zero() && b.is_zero() {
        da * db
    } else {
        da * b + db * a
    };
    result.to_f64().abs()
}

fn propagated_error<T: RealLike>(a: &T, ea: f64, b: &T, eb: f64, divide: bool) -> f64 {
    if !ea.is_finite()
        || ea < 0.
        || !eb.is_finite()
        || eb < 0.
        || !a.is_finite()
        || !b.is_finite()
        || (divide && b.is_zero())
    {
        return f64::NAN;
    }
    let nonzero = if divide {
        ea > 0. || (eb > 0. && !a.is_zero())
    } else if a.is_zero() && b.is_zero() {
        ea > 0. && eb > 0.
    } else {
        (ea > 0. && !b.is_zero()) || (eb > 0. && !a.is_zero())
    };
    if !nonzero {
        return 0.;
    }
    let af = a.to_f64().abs();
    let bf = b.to_f64().abs();
    let converted =
        af.is_finite() && bf.is_finite() && (af != 0. || a.is_zero()) && (bf != 0. || b.is_zero());
    let (left, right, ratios_normal) = if divide {
        let error_ratio = eb / bf;
        let value_ratio = af / bf;
        (
            ea / bf,
            error_ratio * value_ratio,
            (error_ratio.is_normal() || eb == 0.) && (value_ratio.is_normal() || a.is_zero()),
        )
    } else if a.is_zero() && b.is_zero() {
        (ea * eb, 0., true)
    } else {
        (ea * bf, eb * af, true)
    };
    let error = left + right;
    // Normal intermediates away from overflow keep the inexpensive path.
    // Subnormal operands can lose most of their significant bits on conversion;
    // near overflow, even normal conversion rounding can change availability.
    // Recompute these cases using existing native arithmetic.
    if converted
        && (af.is_normal() || a.is_zero())
        && (bf.is_normal() || b.is_zero())
        && ratios_normal
        && error.is_normal()
        && error < f64::MAX / 2.
        && (left.is_normal() || (left == 0. && ea == 0.))
        && (right.is_normal() || (right == 0. && (eb == 0. || a.is_zero())))
    {
        return error;
    }
    let projection_exact = converted
        && magnitude(a) == a.from_rational(&Rational::try_from(af).unwrap())
        && magnitude(b) == b.from_rational(&Rational::try_from(bf).unwrap());
    let error = if projection_exact && a.get_precision() <= 64 && b.get_precision() <= 64 {
        // Even T=f64 needs the wider exponent range for the intermediate sum.
        // Exact operands do not justify reducing the calculation precision:
        // keep higher-precision and exact domains in their native arithmetic.
        native_error(
            &Float::with_val(64, af),
            ea,
            &Float::with_val(64, bf),
            eb,
            divide,
        )
    } else {
        native_error(a, ea, b, eb, divide)
    };
    checked_error(error, true)
}

fn rational_weighted_error(error: f64, weight: &Rational, divide: bool) -> f64 {
    if !error.is_finite() || error < 0. || (divide && weight.is_zero()) {
        return f64::NAN;
    }
    if error == 0. || (!divide && weight.is_zero()) {
        return 0.;
    }
    let scale = weight.to_f64().abs();
    let fast = if divide { error / scale } else { error * scale };
    if fast.is_normal() && fast < f64::MAX / 2. && scale.is_normal() {
        return fast;
    }
    let error = Rational::try_from(error).unwrap();
    let error = if divide {
        error / weight
    } else {
        error * weight
    };
    checked_error(error.to_f64().abs(), true)
}

fn native_relative_error<T: RealLike>(value: &T, mut precision: f64) -> f64 {
    let mut value = magnitude(value);
    if value.fixed_precision() && precision.abs() > f64::MAX_EXP as f64 {
        // Fixed-precision domains include Rational and its wrappers; epsilon
        // is not a reliable exact-domain discriminator. Their powers may
        // materialize integers. Normalize with a bounded native power first,
        // so work depends on the input size,
        // not an arbitrarily large requested decimal precision. Each step moves
        // its magnitude towards the finite nonzero f64 range without narrowing.
        let step = value.from_usize(10).pow(300);
        while value.to_f64() == 0. {
            value *= &step;
            precision += 300.;
        }
        while !value.to_f64().is_finite() {
            value /= &step;
            precision -= 300.;
        }
        // After normalization the magnitude is between roughly 10^-324 and
        // 10^309. A decimal scale beyond +/-1024 cannot give a finite nonzero
        // f64 error. Avoid huge exact powers and the native u32 exponent limit.
        if precision.abs() > f64::MAX_EXP as f64 {
            return f64::NAN;
        }
    }
    // Split off the fractional decimal digit so the large scale uses native
    // integer powers, rather than an already underflowed f64 power of ten.
    let whole = precision.trunc();
    if whole.abs() >= u64::MAX as f64 {
        return f64::NAN;
    }
    let scale = value.from_usize(10).pow(whole.abs() as u64);
    let fraction = value.from_rational(&Rational::try_from(10f64.powf(whole - precision)).unwrap());
    let error = if whole >= 0. {
        value / scale
    } else {
        value * scale
    };
    (error * fraction).to_f64().abs()
}

fn relative_error<T: RealLike>(value: &T, precision: f64) -> f64 {
    if !value.is_finite() || precision.is_nan() || precision == f64::NEG_INFINITY {
        return f64::NAN;
    }
    if precision == f64::INFINITY {
        return 0.;
    }
    let scale = value.to_f64().abs();
    let factor = 10f64.powf(-precision);
    let fast = factor * scale;
    if fast.is_normal() && fast < f64::MAX / 2. && scale.is_normal() && factor.is_normal() {
        return fast;
    }
    let projection_exact = scale.is_finite()
        && scale != 0.
        && magnitude(value) == value.from_rational(&Rational::try_from(scale).unwrap());
    let error = if projection_exact && value.get_precision() <= 64 {
        // Widen the exponent range without reducing native precision.
        native_relative_error(&Float::with_val(64, scale), precision)
    } else {
        native_relative_error(value, precision)
    };
    checked_error(error, true)
}

impl<T: FloatLike + From<f64>> From<f64> for ErrorPropagatingFloat<T> {
    fn from(value: f64) -> Self {
        if value == 0. {
            ErrorPropagatingFloat {
                value: value.into(),
                abs_err: f64::EPSILON,
            }
        } else {
            ErrorPropagatingFloat {
                value: value.into(),
                abs_err: checked_error(f64::EPSILON * value.abs(), true),
            }
        }
    }
}

impl<T: FloatLike> Neg for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        ErrorPropagatingFloat {
            value: -self.value,
            abs_err: self.abs_err,
        }
    }
}

impl<T: FloatLike> Add<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: &Self) -> Self::Output {
        ErrorPropagatingFloat {
            abs_err: self.abs_err + rhs.abs_err,
            value: self.value + &rhs.value,
        }
    }
}

impl<T: FloatLike> Add<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        self + &rhs
    }
}

impl<T: FloatLike> Sub<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: &Self) -> Self::Output {
        self - rhs.clone()
    }
}

impl<T: FloatLike> Sub<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self + (-rhs)
    }
}

impl<T: RealLike> Mul<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: &Self) -> Self::Output {
        ErrorPropagatingFloat {
            abs_err: propagated_error(&self.value, self.abs_err, &rhs.value, rhs.abs_err, false),
            value: self.value * &rhs.value,
        }
    }
}

impl<T: RealLike + Add<Rational, Output = T>, R: Into<Rational>> Add<R>
    for ErrorPropagatingFloat<T>
{
    type Output = Self;

    #[inline]
    fn add(self, rhs: R) -> Self::Output {
        ErrorPropagatingFloat {
            abs_err: self.abs_err,
            value: self.value + rhs.into(),
        }
    }
}

impl<T: RealLike + Add<Rational, Output = T>, R: Into<Rational>> Sub<R>
    for ErrorPropagatingFloat<T>
{
    type Output = Self;

    #[inline]
    fn sub(self, rhs: R) -> Self::Output {
        self + -rhs.into()
    }
}

impl<T: RealLike + Mul<Rational, Output = T>, R: Into<Rational>> Mul<R>
    for ErrorPropagatingFloat<T>
{
    type Output = Self;

    #[inline]
    fn mul(self, rhs: R) -> Self::Output {
        let rhs = rhs.into();
        ErrorPropagatingFloat {
            abs_err: rational_weighted_error(self.abs_err, &rhs, false),
            value: self.value * rhs,
        }
        .truncate()
    }
}

impl<T: RealLike + Div<Rational, Output = T>, R: Into<Rational>> Div<R>
    for ErrorPropagatingFloat<T>
{
    type Output = Self;

    #[inline]
    fn div(self, rhs: R) -> Self::Output {
        let rhs = rhs.into();
        ErrorPropagatingFloat {
            abs_err: rational_weighted_error(self.abs_err, &rhs, true),
            value: self.value.clone() / rhs,
        }
        .truncate()
    }
}

impl<T: FloatLike + From<f64>> Add<f64> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn add(self, rhs: f64) -> Self::Output {
        self + Self::from(rhs)
    }
}

impl<T: FloatLike + From<f64>> Sub<f64> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: f64) -> Self::Output {
        self - Self::from(rhs)
    }
}

impl<T: RealLike + From<f64>> Mul<f64> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: f64) -> Self::Output {
        self * Self::from(rhs)
    }
}

impl<T: RealLike + From<f64>> Div<f64> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn div(self, rhs: f64) -> Self::Output {
        self / Self::from(rhs)
    }
}

impl<T: RealLike> Mul<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        self * &rhs
    }
}

impl<T: RealLike> Div<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn div(self, rhs: &Self) -> Self::Output {
        ErrorPropagatingFloat {
            abs_err: propagated_error(&self.value, self.abs_err, &rhs.value, rhs.abs_err, true),
            value: self.value / &rhs.value,
        }
    }
}

impl<T: RealLike> Div<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    type Output = Self;

    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        self / &rhs
    }
}

impl<T: RealLike> AddAssign<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn add_assign(&mut self, rhs: &ErrorPropagatingFloat<T>) {
        // TODO: optimize
        *self = self.clone() + rhs;
    }
}

impl<T: RealLike> AddAssign<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn add_assign(&mut self, rhs: ErrorPropagatingFloat<T>) {
        self.add_assign(&rhs)
    }
}

impl<T: RealLike> SubAssign<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn sub_assign(&mut self, rhs: &ErrorPropagatingFloat<T>) {
        // TODO: optimize
        *self = self.clone() - rhs;
    }
}

impl<T: RealLike> SubAssign<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn sub_assign(&mut self, rhs: ErrorPropagatingFloat<T>) {
        self.sub_assign(&rhs)
    }
}

impl<T: RealLike> MulAssign<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn mul_assign(&mut self, rhs: &ErrorPropagatingFloat<T>) {
        // TODO: optimize
        *self = self.clone() * rhs;
    }
}

impl<T: RealLike> MulAssign<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn mul_assign(&mut self, rhs: ErrorPropagatingFloat<T>) {
        self.mul_assign(&rhs)
    }
}

impl<T: RealLike> DivAssign<&ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn div_assign(&mut self, rhs: &ErrorPropagatingFloat<T>) {
        // TODO: optimize
        *self = self.clone() / rhs;
    }
}

impl<T: RealLike> DivAssign<ErrorPropagatingFloat<T>> for ErrorPropagatingFloat<T> {
    #[inline]
    fn div_assign(&mut self, rhs: ErrorPropagatingFloat<T>) {
        self.div_assign(&rhs)
    }
}

impl<T: RealLike> ErrorPropagatingFloat<T> {
    /// Create a new precision tracking float with a number of precise decimal digits `prec`.
    /// The `prec` must be smaller than the precision of the underlying float.
    /// This contract is not checked. Positive infinity retains the explicit
    /// zero-error convention; finite inputs never request exactness.
    ///
    /// If the value provided is 0, the precision argument is interpreted as an accuracy (
    /// the number of digits of the absolute error).
    pub fn new(value: T, prec: f64) -> Self {
        let abs_err = if value.is_zero() {
            accuracy_error(prec)
        } else {
            relative_error(&value, prec)
        };
        ErrorPropagatingFloat { value, abs_err }
    }

    /// Return the tracked absolute error. A NaN error denotes an unavailable
    /// uncertainty, including a range failure; zero is reserved for exactness
    /// in the operations documented on this type. Other operations retain their
    /// existing approximate propagation rules.
    pub fn get_absolute_error(&self) -> f64 {
        self.abs_err
    }

    pub fn get_relative_error(&self) -> f64 {
        self.abs_err / self.value.to_f64().abs()
    }

    /// Get the precision in number of decimal digits.
    #[inline(always)]
    pub fn get_precision(&self) -> Option<f64> {
        let r = self.value.to_f64().abs();
        if r == 0. {
            None
        } else {
            Some(-(self.abs_err / r).log10())
        }
    }

    /// Get the accuracy in number of decimal digits.
    #[inline(always)]
    pub fn get_accuracy(&self) -> f64 {
        -self.abs_err.log10()
    }

    /// Truncate the precision to the maximal number of stable decimal digits
    /// of the underlying float.
    #[inline(always)]
    pub fn truncate(mut self) -> Self {
        if self.abs_err.is_finite() && self.value.fixed_precision() {
            self.abs_err = self
                .abs_err
                .max(self.value.get_epsilon() * self.value.to_f64());
        }
        self
    }
}

impl<T: FloatLike> ErrorPropagatingFloat<T> {
    /// Construct with absolute error `10^(-acc)`. Positive infinity explicitly
    /// requests zero error. A finite accuracy whose error is outside the `f64`
    /// range yields a NaN error, rather than silently claiming exactness.
    pub fn new_with_accuracy(value: T, acc: f64) -> Self {
        ErrorPropagatingFloat {
            value,
            abs_err: accuracy_error(acc),
        }
    }

    /// Get the number.
    #[inline(always)]
    pub fn get_num(&self) -> &T {
        &self.value
    }
}

impl<T: RealLike> fmt::Display for ErrorPropagatingFloat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if let Some(p) = self.get_precision() {
            if p < 0. {
                f.write_char('0')
            } else {
                f.write_fmt(format_args!("{0:.1$}", self.value, p as usize))
            }
        } else {
            f.write_char('0')
        }
    }
}

impl<T: RealLike> Debug for ErrorPropagatingFloat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.value, f)?;

        if let Some(p) = self.get_precision() {
            f.write_fmt(format_args!("`{p:.2}"))
        } else {
            f.write_fmt(format_args!("``{:.2}", -self.abs_err.log10()))
        }
    }
}

impl<T: RealLike> LowerExp for ErrorPropagatingFloat<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(self, f)
    }
}

impl<T: FloatLike> PartialEq for ErrorPropagatingFloat<T> {
    fn eq(&self, other: &Self) -> bool {
        // TODO: ignore precision for partial equality?
        self.value == other.value
    }
}

impl<T: FloatLike + PartialOrd> PartialOrd for ErrorPropagatingFloat<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.value.partial_cmp(&other.value)
    }
}

impl<T: RealLike> FloatLike for ErrorPropagatingFloat<T> {
    fn nan(&self) -> Option<Self> {
        Some(Self {
            value: self.value.nan()?,
            abs_err: f64::NAN,
        })
    }

    fn set_from(&mut self, other: &Self) {
        self.value.set_from(&other.value);
        self.abs_err = other.abs_err;
    }

    fn mul_add(&self, a: &Self, b: &Self) -> Self {
        self.clone() * a + b
    }

    fn neg(&self) -> Self {
        -self.clone()
    }

    fn zero(&self) -> Self {
        ErrorPropagatingFloat {
            value: self.value.zero(),
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)),
        }
    }

    fn new_zero() -> Self {
        ErrorPropagatingFloat {
            value: T::new_zero(),
            abs_err: 2f64.powi(-53),
        }
    }

    fn one(&self) -> Self {
        ErrorPropagatingFloat {
            value: self.value.one(),
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)),
        }
    }

    fn pow(&self, e: u64) -> Self {
        let i = self.to_f64().abs();

        if i == 0. {
            return ErrorPropagatingFloat {
                value: self.value.pow(e),
                abs_err: self.abs_err.powf(e as f64),
            };
        }

        let r = self.value.pow(e);
        ErrorPropagatingFloat {
            abs_err: self.abs_err * e as f64 * r.to_f64().abs() / i,
            value: r,
        }
    }

    fn inv(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: propagated_error(&self.value.one(), 0., &self.value, self.abs_err, true),
            value: self.value.inv(),
        }
    }

    /// Convert from a `usize`.
    fn from_usize(&self, a: usize) -> Self {
        let v = self.value.from_usize(a);
        let r = v.to_f64().abs();
        if r == 0. {
            ErrorPropagatingFloat {
                value: v,
                abs_err: 2f64.powf(-(self.value.get_precision() as f64)),
            }
        } else {
            ErrorPropagatingFloat {
                value: v,
                abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * r,
            }
        }
    }

    /// Convert from a `i64`.
    fn from_i64(&self, a: i64) -> Self {
        let v = self.value.from_i64(a);
        let r = v.to_f64().abs();
        if r == 0. {
            ErrorPropagatingFloat {
                value: v,
                abs_err: 2f64.powf(-(self.value.get_precision() as f64)),
            }
        } else {
            ErrorPropagatingFloat {
                value: v,
                abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * r,
            }
        }
    }

    fn get_precision(&self) -> u32 {
        // return the precision of the underlying float instead
        // of the current tracked precision
        self.value.get_precision()
    }

    fn set_precision(&mut self, precision: u32) {
        self.value.set_precision(precision);
        // reset the tracked error to the roundoff of the value
        let scale = self.value.to_f64().abs();
        self.abs_err =
            if scale == 0. { 1. } else { scale } * 2f64.powf(-(self.value.get_precision() as f64));
    }

    fn get_epsilon(&self) -> f64 {
        2.0f64.powi(-(self.value.get_precision() as i32))
    }

    #[inline(always)]
    fn fixed_precision(&self) -> bool {
        self.value.fixed_precision()
    }

    fn sample_unit<R: Rng + ?Sized>(&self, rng: &mut R) -> Self {
        let v = self.value.sample_unit(rng);
        ErrorPropagatingFloat {
            abs_err: self.abs_err * v.to_f64().abs(),
            value: v,
        }
    }

    fn is_fully_zero(&self) -> bool {
        self.value.is_fully_zero()
    }
}

impl<T: RealLike> SingleFloat for ErrorPropagatingFloat<T> {
    fn is_zero(&self) -> bool {
        self.value.is_zero()
    }

    fn is_one(&self) -> bool {
        self.value.is_one()
    }

    fn is_finite(&self) -> bool {
        self.value.is_finite()
    }

    fn from_rational(&self, rat: &Rational) -> Self {
        if rat.is_zero() {
            ErrorPropagatingFloat {
                value: self.value.from_rational(rat),
                abs_err: self.abs_err,
            }
        } else {
            ErrorPropagatingFloat {
                value: self.value.from_rational(rat),
                abs_err: self.abs_err * rat.to_f64(),
            }
        }
    }
}

impl<T: RealLike> RealLike for ErrorPropagatingFloat<T> {
    fn to_usize_clamped(&self) -> usize {
        self.value.to_usize_clamped()
    }

    fn to_f64(&self) -> f64 {
        self.value.to_f64()
    }

    fn round_to_nearest_integer(&self) -> Integer {
        // TODO: what does this do with the error?
        self.value.round_to_nearest_integer()
    }
}

impl<T: Real + RealLike> Real for ErrorPropagatingFloat<T> {
    fn pi(&self) -> Self {
        let v = self.value.pi();
        ErrorPropagatingFloat {
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * v.to_f64(),
            value: v,
        }
    }

    fn e(&self) -> Self {
        let v = self.value.e();
        ErrorPropagatingFloat {
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * v.to_f64(),
            value: v,
        }
    }

    fn euler(&self) -> Self {
        let v = self.value.euler();
        ErrorPropagatingFloat {
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * v.to_f64(),
            value: v,
        }
    }

    fn phi(&self) -> Self {
        let v = self.value.phi();
        ErrorPropagatingFloat {
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)) * v.to_f64(),
            value: v,
        }
    }

    #[inline(always)]
    fn i(&self) -> Option<Self> {
        Some(ErrorPropagatingFloat {
            value: self.value.i()?,
            abs_err: 2f64.powf(-(self.value.get_precision() as f64)),
        })
    }

    fn conj(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err,
            value: self.value.conj(),
        }
    }

    fn norm(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err,
            value: self.value.norm(),
        }
    }

    fn hypot(&self, other: &Self) -> Self {
        let value = self.value.hypot(&other.value);
        if value.is_zero() || !value.is_finite() {
            // Keep the existing arithmetic at the nondifferentiable origin and
            // for exceptional inputs, without inventing an uncertainty bound.
            return (self.clone() * self + other.clone() * other).sqrt();
        }
        // Form bounded derivative weights in the original numeric domain.
        // Squaring either input first can overflow or underflow unnecessarily.
        let x_weight = (self.value.clone() / &value).to_f64().abs();
        let y_weight = (other.value.clone() / &value).to_f64().abs();
        ErrorPropagatingFloat {
            abs_err: self.abs_err * x_weight + other.abs_err * y_weight,
            value,
        }
        .truncate()
    }

    fn sqrt(&self) -> Self {
        let v = self.value.sqrt();
        let r = v.to_f64().abs();

        ErrorPropagatingFloat {
            abs_err: self.abs_err / (2. * r),
            value: v,
        }
        .truncate()
    }

    fn log(&self) -> Self {
        let r = self.value.log();
        ErrorPropagatingFloat {
            abs_err: self.abs_err / self.value.to_f64().abs(),
            value: r,
        }
        .truncate()
    }

    fn exp(&self) -> Self {
        let v = self.value.exp();
        ErrorPropagatingFloat {
            abs_err: v.to_f64().abs() * self.abs_err,
            value: v,
        }
        .truncate()
    }

    fn sin(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err * self.value.to_f64().cos().abs(),
            value: self.value.sin(),
        }
        .truncate()
    }

    fn cos(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err * self.value.to_f64().sin().abs(),
            value: self.value.cos(),
        }
        .truncate()
    }

    fn tan(&self) -> Self {
        let t = self.value.tan();
        let tt = t.to_f64().abs();

        ErrorPropagatingFloat {
            abs_err: self.abs_err * (1. + tt * tt),
            value: t,
        }
        .truncate()
    }

    fn asin(&self) -> Self {
        let v = self.value.to_f64();
        let t = self.value.asin();
        let tt = (1. - v * v).sqrt();
        ErrorPropagatingFloat {
            abs_err: self.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn acos(&self) -> Self {
        let v = self.value.to_f64();
        let t = self.value.acos();
        let tt = (1. - v * v).sqrt();
        ErrorPropagatingFloat {
            abs_err: self.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn atan2(&self, x: &Self) -> Self {
        let t = self.value.atan2(&x.value);
        let r = self.clone() / x;
        let r2 = r.value.to_f64().abs();

        let tt = 1. + r2 * r2;
        ErrorPropagatingFloat {
            abs_err: r.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn sinh(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err * self.value.cosh().to_f64().abs(),
            value: self.value.sinh(),
        }
        .truncate()
    }

    fn cosh(&self) -> Self {
        ErrorPropagatingFloat {
            abs_err: self.abs_err * self.value.sinh().to_f64().abs(),
            value: self.value.cosh(),
        }
        .truncate()
    }

    fn tanh(&self) -> Self {
        let t = self.value.tanh();
        let tt = t.clone().to_f64().abs();
        ErrorPropagatingFloat {
            abs_err: self.abs_err * (1. - tt * tt),
            value: t,
        }
        .truncate()
    }

    fn asinh(&self) -> Self {
        let v = self.value.to_f64();
        let t = self.value.asinh();
        let tt = (1. + v * v).sqrt();
        ErrorPropagatingFloat {
            abs_err: self.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn acosh(&self) -> Self {
        let v = self.value.to_f64();
        let t = self.value.acosh();
        let tt = (v * v - 1.).sqrt();
        ErrorPropagatingFloat {
            abs_err: self.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn atanh(&self) -> Self {
        let v = self.value.to_f64();
        let t = self.value.atanh();
        let tt = 1. - v * v;
        ErrorPropagatingFloat {
            abs_err: self.abs_err / tt,
            value: t,
        }
        .truncate()
    }

    fn powf(&self, e: &Self) -> Self {
        let i = self.to_f64().abs();

        if i == 0. {
            return ErrorPropagatingFloat {
                value: self.value.powf(&e.value),
                abs_err: 0.,
            };
        }

        let r = self.value.powf(&e.value);
        ErrorPropagatingFloat {
            abs_err: (self.abs_err * e.value.to_f64() + i * e.abs_err * i.ln().abs())
                * r.to_f64().abs()
                / i,
            value: r,
        }
        .truncate()
    }
}

impl<T: FloatLike + FixedPrecision> FixedPrecision for ErrorPropagatingFloat<T> {
    const BINARY_PRECISION: usize = T::BINARY_PRECISION;
    const DECIMAL_PRECISION: usize = T::DECIMAL_PRECISION;
}

#[cfg(test)]
mod hypot_tests {
    use crate::domains::float::{
        Complex, DoubleFloat, ErrorPropagatingFloat, Float, FloatLike, Real, RealLike, SingleFloat,
    };

    fn tracked<T: FloatLike>(value: T, accuracy: f64) -> ErrorPropagatingFloat<T> {
        ErrorPropagatingFloat::new_with_accuracy(value, accuracy)
    }

    fn scaled_hypot<T: Real + RealLike>(one: T) {
        let big = one.from_usize(10).pow(200);
        let tiny = big.inv();
        for scale in [big, tiny] {
            for sign in [1, -1] {
                let x = scale.clone() * one.from_i64(sign);
                let y = scale.clone() * one.from_usize(2);
                let expected = x.hypot(&y);
                let a = tracked(x, 220.);
                let b = tracked(y, 220.);
                let value = a.hypot(&b);
                assert!(value.is_finite());
                assert!(((value.get_num().clone() / &expected).to_f64() - 1.).abs() < 1e-14);
                assert!(value.get_absolute_error() > 0.);
                let reversed = b.hypot(&a);
                assert!(((reversed.get_num().clone() / &expected).to_f64() - 1.).abs() < 1e-14);
                let norm = Complex::new(a, b).norm();
                assert!(norm.re.is_finite());
                assert!(((norm.re.get_num().clone() / &expected).to_f64() - 1.).abs() < 1e-14);
            }
        }
    }

    #[test]
    fn tracked_hypot_uses_existing_scaled_native_arithmetic() {
        scaled_hypot(1_f64);
        scaled_hypot(DoubleFloat::from(1.));
        scaled_hypot(Float::with_val(192, 1));
    }

    #[test]
    fn tracked_hypot_retains_local_uncertainty() {
        let value = tracked(3_f64, 6.).hypot(&tracked(4., 6.));
        assert_eq!(*value.get_num(), 5.);
        // Existing linear propagation can overestimate shared intermediates, but
        // must retain both first derivatives, 3/5 and 4/5.
        assert!(value.get_absolute_error() >= 1.4e-6 * (1. - 1e-14));
        let value = tracked(3_f64, 6.).hypot(&tracked(0., 6.));
        assert_eq!(*value.get_num(), 3.);
        assert!(value.get_absolute_error() >= 1e-6);
    }
}

#[cfg(test)]
mod error_range_tests {
    use crate::domains::{
        float::{ErrorPropagatingFloat as E, Float, FloatLike, RealLike, SingleFloat},
        rational::Rational,
    };

    fn p10(e: i32) -> Float {
        let v = Float::with_val(192, 10).pow(e.unsigned_abs() as u64);
        if e < 0 { v.inv() } else { v }
    }
    fn exact(v: Float) -> E<Float> {
        E::new_with_accuracy(v, f64::INFINITY)
    }
    fn near(actual: f64, expected: f64) {
        assert!(
            actual.is_finite() && actual > 0.,
            "expected finite positive {expected:e}, got {actual:e}"
        );
        assert!(
            (actual / expected - 1.).abs() < 1e-12,
            "expected {expected:e}, got {actual:e}"
        );
    }
    #[test]
    fn valid_precision_epf_weight_underflow_is_not_exact() {
        let a = E::new(p10(-200), 10.);
        near(a.get_absolute_error(), 1e-210);
        let v = a * exact(p10(-200));
        assert!(!v.get_num().is_zero());
        assert!(
            v.get_absolute_error().is_nan(),
            "lost nonzero ~1e-410 uncertainty: {}",
            v.get_absolute_error()
        );
    }
    #[test]
    fn valid_precision_rational_weight_underflow_is_not_exact() {
        let a = E::new(p10(-200), 10.);
        let w = Rational::from(10).pow(200).inv();
        let v = a * w;
        assert!(!v.get_num().is_zero());
        assert!(v.get_absolute_error().is_nan());
    }
    #[test]
    fn finite_composite_errors_survive_wide_epf_weights() {
        let a = E::new_with_accuracy(Float::with_val(192, 1), 200.);
        near((a.clone() * exact(p10(400))).get_absolute_error(), 1e200);
        near((a / exact(p10(-400))).get_absolute_error(), 1e200);
    }
    #[test]
    fn finite_composite_errors_survive_wide_rational_weights() {
        let a = E::new_with_accuracy(Float::with_val(192, 1), 200.);
        let w = Rational::from(10).pow(400);
        near((a.clone() * w.clone()).get_absolute_error(), 1e200);
        near((a / w.inv()).get_absolute_error(), 1e200);
    }
    #[test]
    fn valid_constructor_tiny_native_center_is_not_zero_center() {
        let a = E::new(p10(-400), 10.);
        assert!(!a.get_num().is_zero());
        assert!(a.get_absolute_error().is_nan());
    }
    #[test]
    fn reciprocal_retains_representable_small_error() {
        let a = E::new_with_accuracy(p10(200), -100.);
        near(a.inv().get_absolute_error(), 1e-300);
    }
    #[test]
    fn explicit_accuracy_range_failure_survives_truncate() {
        let a = E::new_with_accuracy(1_f64, 400.).truncate();
        assert!(a.get_absolute_error().is_nan());
    }
    #[test]
    fn exact_controls_stay_exact() {
        let a = exact(p10(-200));
        assert_eq!((a.clone() * exact(p10(400))).get_absolute_error(), 0.);
        assert_eq!((a.clone() / exact(p10(-400))).get_absolute_error(), 0.);
        assert_eq!((a * Rational::from(10).pow(400)).get_absolute_error(), 0.);
        let a = E::new(p10(-200), 10.);
        assert_eq!(
            (a * exact(Float::with_val(192, 0))).get_absolute_error(),
            0.
        );
    }
    #[test]
    fn quotient_retains_denominator_uncertainty_before_narrowing() {
        let x = E::new(p10(300), 50.);
        let q = x.clone() / x;
        near(q.get_absolute_error(), 2e-50);
        assert_eq!(q.get_num().to_f64(), 1.);
        let x = E::new(1e-310_f64, 10.);
        let expected = 2. * x.get_absolute_error() / 1e-310;
        let q = x / x;
        assert_eq!(*q.get_num(), 1.);
        near(q.get_absolute_error(), expected);
    }
    #[test]
    fn two_subnormal_contributions_are_narrowed_after_sum() {
        let ea = f64::from_bits(1);
        let a = E::new_with_accuracy(0.5_f64, -ea.log10());
        assert_eq!(a.get_absolute_error(), ea);
        let out = a * a;
        assert_eq!(out.get_absolute_error(), ea);
    }
    #[test]
    fn representable_subnormal_error_is_kept() {
        let a = E::new_with_accuracy(Float::with_val(192, 1), 300.);
        near((a * exact(p10(-20))).get_absolute_error(), 1e-320);
    }
    #[test]
    fn constructor_preserves_representable_composite_error() {
        let value = Float::with_val(2048, 10).pow(200);
        near(E::new(value, 400.).get_absolute_error(), 1e-200);
        near(E::new(p10(310), 10.).get_absolute_error(), 1e300);
    }
    #[test]
    fn unknown_uncertainty_is_not_recovered_by_zero_weight_or_truncate() {
        let unknown = E::new_with_accuracy(1_f64, 400.);
        assert!(unknown.get_absolute_error().is_nan());
        assert!(
            (unknown * E::new_with_accuracy(0., f64::INFINITY))
                .get_absolute_error()
                .is_nan()
        );
        let unknown_native = E::new_with_accuracy(Float::with_val(192, 1), 400.);
        assert!(
            (unknown_native.clone() * Rational::from(0))
                .get_absolute_error()
                .is_nan()
        );
        assert!(
            (unknown_native / Rational::from(2))
                .get_absolute_error()
                .is_nan()
        );
        assert!(unknown.truncate().get_absolute_error().is_nan());
    }
    #[test]
    fn ordinary_signed_weighting_is_unchanged() {
        for sign in [1, -1] {
            let a = E::new_with_accuracy(Float::with_val(192, sign * 3), 6.);
            near(
                (a.clone() * Rational::from(sign * 2)).get_absolute_error(),
                2e-6,
            );
            near(
                (a.clone() / Rational::from(sign * 2)).get_absolute_error(),
                5e-7,
            );
            near(
                (a.clone() * exact(Float::with_val(192, sign * 2))).get_absolute_error(),
                2e-6,
            );
            near(
                (a / exact(Float::with_val(192, sign * 2))).get_absolute_error(),
                5e-7,
            );
        }
    }
    #[test]
    fn out_of_contract_precision_fixture_does_not_silently_become_exact() {
        // 400 decimal digits exceed the 192-bit native precision contract.
        // This is defensive fixture behavior, not valid-input bug evidence.
        let a = E::new(Float::with_val(192, 0), 400.);
        assert!(a.get_absolute_error().is_nan());
    }

    #[test]
    fn inexact_weight_projection_must_not_reject_finite_final_error() {
        let min = f64::from_bits(1);
        let a = E::new_with_accuracy(Float::parse("1e-300", Some(192)).unwrap(), -min.log10());
        assert_eq!(a.get_absolute_error(), min);
        let rational = Rational::from((1, 2)) + Rational::from(2).pow(100).inv();
        let weight = rational.to_multi_prec_float(192);
        assert_eq!(weight.to_f64(), 0.5);
        let native_error = weight.clone() * Rational::try_from(min).unwrap();
        assert_eq!(native_error.to_f64(), min);
        let out = a * E::new_with_accuracy(weight, f64::INFINITY);
        assert_eq!(out.get_absolute_error(), min);
    }
    #[test]
    fn subnormal_decimal_scale_does_not_lose_finite_composite_accuracy() {
        let value = Float::with_val(2048, 10).pow(308);
        let error = E::new(value, 323.).get_absolute_error();
        assert!((error / 1e-15 - 1.).abs() < 1e-12, "{error:e}");
    }

    #[test]
    fn native_subnormal_weight_keeps_finite_quotient_error() {
        let a = E::new(Float::with_val(192, 1), 15.);
        let weight = Float::parse("7e-324", Some(192)).unwrap();
        let expected = (Float::with_val(192, a.get_absolute_error()) / &weight).to_f64();
        let out = a / exact(weight);
        near(out.get_absolute_error(), expected);
    }
    #[test]
    fn subnormal_internal_division_ratio_is_recomputed() {
        let a = E::new_with_accuracy(Float::parse("1e300", Some(2048)).unwrap(), f64::INFINITY);
        let b = E::new_with_accuracy(Float::parse("1.5e123", Some(2048)).unwrap(), 200.);
        near((a / b).get_absolute_error(), 4.444444444444444e-147);
    }
    #[test]
    fn rational_constructor_bounds_requested_power_work() {
        let tiny = Rational::from(10).pow(400).inv();
        assert!(E::new(tiny, -1e12).get_absolute_error().is_nan());
        for value in [
            Rational::from(10).pow(1000),
            Rational::from(10).pow(1000).inv(),
        ] {
            let precision = if value.to_f64() == 0. { -1000. } else { 1000. };
            let out = E::new(value.clone(), precision);
            assert_eq!(out.get_num(), &value);
            near(out.get_absolute_error(), 1.);
        }
    }
    #[test]
    fn scalar_construction_overflow_and_zero_center_controls() {
        let small = E::<f64>::from(f64::from_bits(1));
        assert!(small.get_absolute_error().is_nan());
        assert_eq!(E::<f64>::from(0.).get_absolute_error(), f64::EPSILON);
        let zero = E::new_with_accuracy(Float::with_val(192, 0), 200.);
        assert!((zero.clone() * zero).get_absolute_error().is_nan());
        let zero = E::new_with_accuracy(Float::with_val(192, 0), 2.);
        near((zero.clone() * zero).get_absolute_error(), 1e-4);
        let large_error = E::new_with_accuracy(Float::with_val(192, 1), -200.);
        assert!(
            (large_error * exact(p10(200)))
                .get_absolute_error()
                .is_nan()
        );
    }
    #[test]
    fn subnormal_projected_divisor_does_not_hide_error_overflow() {
        let a = E::new(Float::with_val(192, 0), 15.25);
        let weight = Float::parse("2.5e-324", Some(192)).unwrap();
        assert!(
            (Float::with_val(192, a.get_absolute_error()) / &weight)
                .to_f64()
                .is_infinite()
        );
        let out = a / E::new_with_accuracy(weight, f64::INFINITY);
        assert!(out.get_absolute_error().is_nan());
    }
    #[test]
    fn subnormal_projected_weight_keeps_native_error_magnitude() {
        let a = E::new(Float::parse("1e308", Some(192)).unwrap(), 8.);
        let weight = Float::parse("7e-324", Some(192)).unwrap();
        let expected = (Float::with_val(192, a.get_absolute_error()) * &weight).to_f64();
        let out = a * E::new_with_accuracy(weight, f64::INFINITY);
        assert!((out.get_absolute_error() / expected - 1.).abs() < 1e-12);
    }
    #[test]
    fn nested_rational_constructor_does_not_materialize_unbounded_power() {
        let center = E::new_with_accuracy(Rational::from((1, 3)), f64::INFINITY);
        let out = E::new(center, -1e12);
        assert!(out.get_absolute_error().is_nan());
        assert_eq!(out.get_num().get_num(), &Rational::from((1, 3)));
        assert_eq!(out.get_num().get_absolute_error(), 0.);
    }
    #[test]
    fn normal_projected_centers_do_not_hide_error_overflow() {
        let weight = Float::with_val(192, f64::MAX) + Float::with_val(192, 2).pow(969);
        assert_eq!(weight.to_f64(), f64::MAX);
        let a = E::new_with_accuracy(weight.clone(), -0.75_f64.log10());
        let eb = 0.25 + 2_f64.powi(-54);
        let b = E::new_with_accuracy(weight.clone(), -eb.log10());
        let expected = Float::with_val(192, a.get_absolute_error()) * &weight
            + Float::with_val(192, b.get_absolute_error()) * &weight;
        assert!(
            expected.to_f64().is_infinite(),
            "fixture errors: {} {}",
            a.get_absolute_error(),
            b.get_absolute_error()
        );
        assert!((a * b).get_absolute_error().is_nan());
    }

    #[test]
    fn rational_power_work_guard_preserves_compensated_errors() {
        let tiny = Rational::from(10).pow(1000).inv();
        near(E::new(tiny, -1025.).get_absolute_error(), 1e25);
        let center = E::new_with_accuracy(Rational::from((1, 3)), f64::INFINITY);
        let nested = E::new_with_accuracy(center, f64::INFINITY);
        let out = E::new(nested, -1e12);
        assert!(out.get_absolute_error().is_nan());
        assert_eq!(out.get_num().get_num().get_num(), &Rational::from((1, 3)));
    }

    #[test]
    fn exact_projections_retain_native_product_precision() {
        let min = f64::from_bits(1);
        let a = E::new_with_accuracy(Float::with_val(192, f64::MIN_POSITIVE), -min.log10());
        let b = E::new_with_accuracy(Float::with_val(192, 0.5), -2_f64.powi(-152).log10());
        assert_eq!(a.get_absolute_error(), min);
        let native = Float::with_val(192, a.get_absolute_error()) * b.get_num()
            + Float::with_val(192, b.get_absolute_error()) * a.get_num();
        assert_eq!(native.to_f64(), min);
        let out = a * b;
        assert_eq!(out.get_absolute_error(), min);
    }

    #[test]
    fn exact_projections_retain_native_quotient_precision() {
        let min = f64::from_bits(1);
        let a = E::new_with_accuracy(Float::with_val(192, f64::MIN_POSITIVE), -min.log10());
        let b = E::new_with_accuracy(Float::with_val(192, 2), -2_f64.powi(-150).log10());
        assert_eq!(a.get_absolute_error(), min);
        let native = Float::with_val(192, a.get_absolute_error()) / b.get_num()
            + (Float::with_val(192, b.get_absolute_error()) / b.get_num())
                * (a.get_num().clone() / b.get_num());
        assert_eq!(native.to_f64(), min);
        let out = a / b;
        assert_eq!(out.get_absolute_error(), min);
    }
}
