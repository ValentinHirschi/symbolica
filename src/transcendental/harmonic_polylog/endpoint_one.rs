//! The two endpoint-one GPL words required by four-loop vacuum periods.
//!
//! This is the ordinary G convention, with kernel 1/(t-a), on the real path
//! from zero to one. No HPL sign conversion is applied.

use super::*;
use crate::atom::AtomCore;

/// Exact letters admitted by the endpoint-one vacuum-period GPL evaluator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GplLetter {
    Zero,
    One,
    /// (-1-i sqrt(3))/2, with positive real sqrt(3).
    R4,
}

static GPL: SymbolCache<Symbol> =
    SymbolCache::new(|| get_symbol!("gpl_at_one").expect("endpoint-one GPL was initialized"));
static LETTERS: SymbolCache<Symbol> =
    SymbolCache::new(|| get_symbol!("gpl_letters").expect("GPL letter tag was initialized"));

#[cfg(not(doctest))]
crate::_inventory::submit! {
    StateInitializer::new("symbolica::endpoint_one_gpl", || {
        let _ = symbol!("gpl_letters";);
        let _ = symbol!(
            "gpl_at_one",
            eval = EvaluationInfo::constant(|tags, bits| {
                if !(2..=MAX_BITS).contains(&bits) {
                    return Err(format!("endpoint-one GPL precision must be between 2 and {MAX_BITS} bits"));
                }
                let word = parse_tags(tags)?;
                let work = bits.checked_add(64).ok_or("GPL precision overflow")?;
                let mut value = EndpointOneGplEvaluator::new(&word)?.evaluate_ball(work)?;
                value.re.center.set_prec(bits);
                value.im.center.set_prec(bits);
                Ok(Complex::new(value.re.center, value.im.center))
            }).with_tags(1).register_tracked(|tags, arguments, bits| {
                if !arguments.is_empty() {
                    return Err("endpoint-one GPL expects one symbolic letter tag".into());
                }
                let word = parse_tags(tags)?;
                Ok(TrackedComplex::from_ball(
                    EndpointOneGplEvaluator::new(&word)?.evaluate_ball(bits)?,
                    ErrorQuality::Bounded,
                ))
            })
        );
    }, &["symbolica"])
}

/// The tagged constant `gpl_at_one(gpl_letters(a1,...,a4))`.
///
/// Initially supports G(0,0,r4,1;1) and G(0,1,1,r4;1), using the ordinary
/// kernel 1/(t-a). Other words are rejected explicitly.
pub fn gpl_at_one() -> Symbol {
    *GPL
}

fn r4() -> Atom {
    (Atom::num(-1)
        - Atom::num(Complex::new(Rational::zero(), Rational::one()))
            * Atom::num(3).pow(Atom::num((1, 2))))
        / 2
}

/// Construct one of the supported endpoint-one GPL constants.
pub fn endpoint_one_gpl(word: &[GplLetter]) -> Result<Atom, String> {
    validate(word)?;
    let letters = word
        .iter()
        .map(|letter| match letter {
            GplLetter::Zero => Atom::num(0),
            GplLetter::One => Atom::num(1),
            GplLetter::R4 => r4(),
        })
        .collect::<Vec<_>>();
    Ok(GPL.call(LETTERS.call(&letters)))
}

fn validate(word: &[GplLetter]) -> Result<(), String> {
    use GplLetter::*;
    if word == [Zero, Zero, R4, One] || word == [Zero, One, One, R4] {
        Ok(())
    } else {
        Err(
            "endpoint-one GPL supports only G(0,0,r4,1;1) and G(0,1,1,r4;1), r4=(-1-i sqrt(3))/2"
                .into(),
        )
    }
}

fn parse_tags(tags: &[AtomView]) -> Result<Vec<GplLetter>, String> {
    let [AtomView::Fun(letters)] = tags else {
        return Err("endpoint-one GPL requires gpl_letters(...) as its symbolic tag".into());
    };
    if letters.get_symbol() != *LETTERS {
        return Err("endpoint-one GPL requires gpl_letters(...) as its symbolic tag".into());
    }
    let root = r4();
    let word = letters
        .iter()
        .map(|letter| {
            if letter == Atom::num(0).as_view() {
                Ok(GplLetter::Zero)
            } else if letter == Atom::num(1).as_view() {
                Ok(GplLetter::One)
            } else if letter == root.as_view() {
                Ok(GplLetter::R4)
            } else {
                Err(format!("unsupported endpoint-one GPL letter: {letter}"))
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate(&word)?;
    Ok(word)
}

/// A shared-suffix Hölder evaluation of one supported endpoint-one GPL.
pub struct EndpointOneGplEvaluator {
    nodes: Vec<Node>,
    terms: Vec<(usize, usize, i8)>,
}

impl EndpointOneGplEvaluator {
    pub fn new(word: &[GplLetter]) -> Result<Self, String> {
        validate(word)?;
        // Pole::Minus is conjugate(r4). Evaluate in that exact quadratic field
        // using the existing recurrence, then conjugate the complete result.
        let poles = word
            .iter()
            .map(|letter| match letter {
                GplLetter::Zero => Pole::Zero,
                GplLetter::One => Pole::One,
                GplLetter::R4 => Pole::Minus,
            })
            .collect::<Vec<_>>();
        let mut nodes = Vec::new();
        let mut indices = BTreeMap::from([(Vec::new(), 0)]);
        let terms = (0..=word.len())
            .map(|split| {
                let left = poles[..split]
                    .iter()
                    .rev()
                    .map(|p| p.complement())
                    .collect::<Vec<_>>();
                (
                    intern_poles(&left, &mut nodes, &mut indices),
                    intern_poles(&poles[split..], &mut nodes, &mut indices),
                    if split % 2 == 0 { 1 } else { -1 },
                )
            })
            .collect();
        Ok(Self { nodes, terms })
    }

    /// Evaluate once at the given working precision, preserving error radii.
    pub fn evaluate_ball(&self, bits: u32) -> Result<ComplexBall, String> {
        if !(2..=MAX_BITS).contains(&bits) {
            return Err(format!(
                "endpoint-one GPL precision must be between 2 and {MAX_BITS} bits"
            ));
        }
        let values = evaluate_gpl_nodes(&self.nodes, bits)?;
        let zero = RealBall::exact(Float::new(bits));
        let mut value = Complex::new(zero.clone(), zero);
        for &(left, right, sign) in &self.terms {
            let product = &values[left] * &values[right];
            if sign == 1 {
                value += product;
            } else {
                value -= product;
            }
        }
        Ok(value.conj())
    }
}

#[cfg(test)]
mod tests;
