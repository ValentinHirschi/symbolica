use numerica::domains::float::{ErrorPropagatingFloat as E, Float, Real};

#[test]
fn hypot_refuses_lost_native_or_converted_weights() {
    let x = E::new_with_accuracy(Float::with_val(512, 1e-200), 210.);
    let y = E::new_with_accuracy(Float::with_val(512, 1e200), f64::INFINITY);
    let result = x.hypot(&y);
    assert!(result.get_absolute_error().is_nan());
    assert!(
        (result * E::new_with_accuracy(Float::with_val(512, 1e200), f64::INFINITY))
            .get_absolute_error()
            .is_nan()
    );
    let x = E::new_with_accuracy(1e-200f64, 210.);
    let y = E::new_with_accuracy(1e200f64, f64::INFINITY);
    assert!(x.hypot(&y).get_absolute_error().is_nan());
    let bad = E::new_with_accuracy(0f64, 400.);
    assert!(
        bad.hypot(&E::new_with_accuracy(1f64, 20.))
            .get_absolute_error()
            .is_nan()
    );
}

#[test]
fn hypot_preserves_stable_ordinary_small_and_large_inputs() {
    for scale in [1e-200f64, 1., 1e200] {
        let accuracy = 6. - scale.log10();
        let x = E::new_with_accuracy(3f64 * scale, accuracy);
        let y = E::new_with_accuracy(4f64 * scale, accuracy);
        let result = x.hypot(&y);
        assert!((result.get_absolute_error() / (1.4e-6 * scale) - 1.).abs() < 1e-12);
    }
    let zero = E::new_with_accuracy(0f64, f64::INFINITY);
    let nonzero = E::new_with_accuracy(2f64, 6.);
    assert_eq!(zero.hypot(&nonzero).get_absolute_error(), 1e-6);
}
