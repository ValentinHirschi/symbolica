//! Safe MathML tokens for coefficient domains without a structural renderer.
use super::{ColorMode, PrintMode, PrintOptions, PrintState};
use crate::domains::Ring;

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn token(tag: &str, text: &str) -> String {
    format!("<{tag}>{}</{tag}>", escape(text))
}

/// Preserve exact domain notation as text, while giving ordinary numbers,
/// signs and rational coefficients their mathematical structure.
pub fn coefficient<R: Ring, W: std::fmt::Write>(
    ring: &R,
    element: &R::Element,
    opts: &PrintOptions,
    state: PrintState,
    f: &mut W,
) -> Result<bool, std::fmt::Error> {
    let mut plain = opts.clone();
    plain.mode = PrintMode::Symbolica;
    plain.color_mode = ColorMode::Never;
    plain.num_exp_as_superscript = false;
    plain.max_line_length = None;
    struct BoundedText(String);
    impl std::fmt::Write for BoundedText {
        fn write_str(&mut self, value: &str) -> std::fmt::Result {
            if self.0.len().saturating_add(value.len()) > 4096 {
                return Err(std::fmt::Error);
            }
            self.0.push_str(value);
            Ok(())
        }
    }
    let mut text = BoundedText(String::new());
    let Ok(suppressed) = ring.format(element, &plain, state, &mut text) else {
        if state.in_sum {
            f.write_str("<mo form=\"infix\">+</mo>")?;
        }
        f.write_str("<mtext>⋯</mtext>")?;
        return Ok(false);
    };
    let text = text.0;
    let (sign, body) = if let Some(rest) = text.strip_prefix('-') {
        (
            if state.in_sum {
                "<mo form=\"infix\">−</mo>"
            } else {
                "<mo>−</mo>"
            },
            rest,
        )
    } else if let Some(rest) = text.strip_prefix('+') {
        ("<mo form=\"infix\">+</mo>", rest)
    } else {
        ("", text.as_str())
    };
    f.write_str(sign)?;
    if !body.is_empty() {
        let integer = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        if integer(body) {
            f.write_str(&token("mn", body))?;
        } else if let Some((num, den)) = body.split_once('/')
            && integer(num)
            && integer(den)
        {
            write!(f, "<mfrac>{}{}</mfrac>", token("mn", num), token("mn", den))?;
        } else {
            f.write_str(&token("mtext", body))?;
        }
    }
    Ok(suppressed)
}
