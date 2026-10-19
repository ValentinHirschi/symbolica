//! Bounded native MathML presentation. Omitted subtrees are display metadata,
//! never substitutions in the underlying expression.
use super::*;
use crate::atom::ListIterator;

pub(crate) const DEFAULT_PAGE_SIZE: usize = 25;
// Bounded alternatives let each notebook view fit locally without kernel traffic.
pub(crate) const LAYOUTS: [(usize, usize); 8] = [
    (500, 32),
    (32, 24),
    (16, 20),
    (8, 16),
    (4, 12),
    (2, 8),
    (1, 4),
    (1, 2),
];
const MAX_NODES: usize = 1_000;
const MAX_DEPTH: usize = 32;
const MAX_LEAF_BYTES: usize = 4_096;
const MAX_TEXT_BYTES: usize = 16_384;
const PRODUCT_ROW_COST: usize = 48;

pub(crate) fn children(atom: AtomView<'_>) -> Option<ListIterator<'_>> {
    match atom {
        AtomView::Add(a) => Some(a.iter()),
        AtomView::Mul(a) => Some(a.iter()),
        AtomView::Fun(a) => Some(a.iter()),
        AtomView::Pow(a) => Some(a.iter()),
        _ => None,
    }
}

pub(crate) fn child_count(atom: AtomView<'_>) -> usize {
    match atom {
        AtomView::Add(a) => a.get_nargs(),
        AtomView::Mul(a) => a.get_nargs(),
        AtomView::Fun(a) => a.get_nargs(),
        AtomView::Pow(_) => 2,
        _ => 0,
    }
}

pub(crate) struct MathMlPage {
    pub html: String,
    pub start: usize,
    pub end: usize,
    pub total: usize,
    pub markers: Vec<MathMlMarker>,
}

pub(crate) struct MathMlMarker {
    pub path: Vec<usize>,
    pub label: String,
    pub start: usize,
}

impl MathMlPage {
    pub fn is_partial(&self) -> bool {
        self.start != 0 || self.end < self.total || !self.markers.is_empty()
    }
}

struct Renderer<'a> {
    opts: &'a PrintOptions,
    nodes: usize,
    bytes: usize,
    budget_exhausted: bool,
    nested_limit: usize,
    depth_limit: usize,
    expansions: &'a [(Vec<usize>, usize)],
    multiline_sums: bool,
    markers: Vec<MathMlMarker>,
}

fn row(body: String) -> String {
    format!("<mrow>{body}</mrow>")
}
fn fence(body: String) -> String {
    row(format!("<mo>(</mo>{body}<mo>)</mo>"))
}
fn token(tag: &str, text: &str) -> String {
    format!("<{tag}>{}</{tag}>", AnsiHtmlFormatter::escape_html(text))
}

// A bounded estimate for grouping already-rendered factors into native MathML
// rows. Count visible text, treating entities as one glyph and ignoring tags.
fn visible_cost(fragment: &str) -> usize {
    let mut cost = 0;
    let mut tag = false;
    let mut entity = false;
    for c in fragment.chars() {
        match c {
            '<' => tag = true,
            '>' => tag = false,
            _ if tag => (),
            '&' => entity = true,
            ';' if entity => {
                entity = false;
                cost += 1;
            }
            _ if entity => (),
            _ => cost += 1,
        }
        if cost > PRODUCT_ROW_COST {
            break;
        }
    }
    cost
}

fn fraction(
    numerator: String,
    denominator: String,
    parts: &mut Option<(String, String)>,
) -> String {
    *parts = Some((numerator.clone(), denominator.clone()));
    format!("<mfrac>{numerator}{denominator}</mfrac>")
}

fn leading_minus(fragment: &str) -> Option<usize> {
    let mut offset = 0;
    while fragment[offset..].starts_with("<mrow>") || fragment[offset..].starts_with("<mrow ") {
        offset += fragment[offset..].find('>')? + 1;
    }
    fragment[offset..]
        .starts_with("<mo>−</mo>")
        .then_some(offset)
}

fn product_sign(mut fragment: String, negative: &mut bool) -> String {
    if let Some(offset) = leading_minus(&fragment) {
        *negative = !*negative;
        fragment.replace_range(offset..offset + "<mo>−</mo>".len(), "");
    }
    fragment
}

fn complex_coefficient(atom: AtomView<'_>) -> bool {
    let AtomView::Num(n) = atom else {
        return false;
    };
    match n.get_coeff_view() {
        CoefficientView::Natural(r, _, i, _) => r != 0 && i != 0,
        CoefficientView::Large(r, i) => !r.is_zero() && !i.is_zero(),
        CoefficientView::Float(r, i) => !r.is_zero() && !i.is_zero(),
        _ => false,
    }
}

// der(d1,...,dn,f,x1,...,xn) is displayed as f^(d1,...,dn)(x1,...,xn).
// Page derivative orders and their corresponding arguments together.
fn derivative_head(atom: AtomView<'_>) -> Option<(usize, Symbol)> {
    let AtomView::Fun(f) = atom else { return None };
    let count = f.get_nargs();
    if f.get_symbol() == Symbol::DERIVATIVE && count >= 3 && count % 2 == 1 {
        f.get(count / 2).get_symbol().map(|s| (count / 2, s))
    } else {
        None
    }
}

fn item_count(atom: AtomView<'_>) -> usize {
    derivative_head(atom).map_or_else(|| child_count(atom), |(n, _)| n)
}

impl Renderer<'_> {
    fn checkpoint(&self) -> (usize, usize, usize, bool) {
        (
            self.nodes,
            self.bytes,
            self.markers.len(),
            self.budget_exhausted,
        )
    }

    fn restore(&mut self, state: (usize, usize, usize, bool)) {
        self.nodes = state.0;
        self.bytes = state.1;
        self.markers.truncate(state.2);
        self.budget_exhausted = state.3;
    }

    fn tagged(body: String, path: &[usize]) -> String {
        let path = path
            .iter()
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join("/");
        format!("<mrow data-symbolica-path=\"{path}\">{body}</mrow>")
    }

    // A marker is roughly four short glyphs wide, including its padding. Probe
    // only that much structure: large trees and coefficients are never printed
    // just to decide whether abbreviating them would save space.
    fn compact_cost(&self, atom: AtomView<'_>, budget: usize) -> Option<usize> {
        if budget == 0 {
            return None;
        }
        let name_cost = |symbol: Symbol| {
            if symbol.get_name().len() > MAX_LEAF_BYTES {
                return None;
            }
            let mut name = String::new();
            symbol
                .format_with_custom(self.opts, PrintState::new(), false, &mut name)
                .ok()?;
            let cost = name.chars().take(budget + 1).count().max(1);
            (cost <= budget).then_some(cost)
        };
        let mut cost = match atom {
            AtomView::Var(v) => return name_cost(v.get_symbol()),
            AtomView::Num(n) => {
                let CoefficientView::Natural(r, 1, 0, 1) = n.get_coeff_view() else {
                    return None;
                };
                let cost = r.to_string().len();
                return (cost <= budget).then_some(cost);
            }
            AtomView::Fun(f) => name_cost(f.get_symbol())? + 2,
            _ => 1,
        };
        if cost > budget || child_count(atom) > budget {
            return None;
        }
        for (i, child) in children(atom)?.enumerate() {
            if i > 0 && !matches!(atom, AtomView::Pow(_)) {
                cost += 1;
            }
            cost += self.compact_cost(child, budget.checked_sub(cost)?)?;
        }
        (cost <= budget).then_some(cost)
    }

    fn compact_tail_limit(&self, atom: AtomView<'_>, start: usize, limit: usize) -> usize {
        let total = child_count(atom);
        let end = start.saturating_add(limit);
        if total <= end || total - end > 4 || total - start > 500 {
            return limit;
        }
        let mut cost = 0;
        for (i, child) in children(atom).unwrap().skip(end).enumerate() {
            if i > 0 {
                cost += 1;
            }
            let Some(child_cost) = self.compact_cost(child, 4_usize.saturating_sub(cost)) else {
                return limit;
            };
            cost += child_cost;
        }
        total - start
    }

    fn hole(&mut self, atom: AtomView<'_>, path: &[usize]) -> String {
        let marker = self.hole_at(atom, path, 0);
        // A collapsed call still exposes its head, while the marker opens its
        // arguments. Reserve the same name budget as an ordinary function.
        if let AtomView::Fun(f) = atom
            && f.get_symbol().get_name().len() <= MAX_LEAF_BYTES
            && self.bytes >= MAX_LEAF_BYTES
        {
            return Self::tagged(
                row(format!(
                    "{}<mo>&#x2061;</mo>{}",
                    self.name(f.get_symbol()),
                    fence(marker)
                )),
                path,
            );
        }
        marker
    }

    fn hole_at(&mut self, atom: AtomView<'_>, path: &[usize], start: usize) -> String {
        // Every omission retains a path, including an oversized indivisible leaf.
        let label = match atom {
            AtomView::Add(_) => "sum",
            AtomView::Mul(_) => "product",
            AtomView::Fun(_) => "function",
            AtomView::Pow(_) => "power",
            _ => "value exceeds display budget",
        };
        let index = self.markers.len();
        self.markers.push(MathMlMarker {
            path: path.to_vec(),
            label: format!("Inspect {label} [{}]", index + 1),
            start,
        });
        // The surrounding call supplies the function's type and name. An
        // abbreviated argument list reads F(x, [1]), without a generic label.
        if matches!(atom, AtomView::Fun(_))
            || (start == 0 && matches!(atom, AtomView::Add(_) | AtomView::Mul(_)))
        {
            return format!(
                "<mtext data-symbolica-marker=\"{index}\">[{}]</mtext>",
                index + 1
            );
        }
        let marker_type = match atom {
            AtomView::Add(_) => "+",
            AtomView::Mul(_) => "·",
            _ => {
                if children(atom).is_some() {
                    label
                } else {
                    "⋯"
                }
            }
        };
        format!(
            "<mrow><mo form=\"infix\">{marker_type}</mo><mtext data-symbolica-marker=\"{index}\">[{}]</mtext></mrow>",
            index + 1
        )
    }

    fn name(&mut self, symbol: Symbol) -> String {
        let mut name = String::new();
        symbol
            .format_with_custom(self.opts, PrintState::new(), false, &mut name)
            .unwrap();
        self.bytes = self.bytes.saturating_sub(name.len());
        token("mi", &name)
    }

    fn render(&mut self, atom: AtomView<'_>, path: &mut Vec<usize>, depth: usize) -> String {
        self.render_factor(atom, path, depth, &mut None)
    }

    fn render_factor(
        &mut self,
        atom: AtomView<'_>,
        path: &mut Vec<usize>,
        depth: usize,
        parts: &mut Option<(String, String)>,
    ) -> String {
        // Explicit expansion reveals a little more depth along this branch.
        // Both the absolute depth and the existing node/text budgets still apply.
        let depth_limit = self
            .expansions
            .iter()
            .filter(|(p, _)| p.starts_with(path) || path.starts_with(p))
            .fold(self.depth_limit, |limit, (p, _)| {
                limit.max((p.len() + 4).min(64))
            });
        if self.nodes == 0 || self.bytes < MAX_LEAF_BYTES {
            self.budget_exhausted = true;
            return self.hole(atom, path);
        }
        if (depth >= depth_limit
            && children(atom).is_some()
            && (depth >= 64
                || if matches!(atom, AtomView::Fun(_)) {
                    // F([1]) retains the head, so only the arguments count
                    // toward the space saved, even for a long function name.
                    self.compact_tail_limit(atom, 0, 0) < child_count(atom)
                } else {
                    self.compact_cost(atom, 4).is_none()
                }))
            || atom
                .get_symbol()
                .is_some_and(|s| s.get_name().len() > MAX_LEAF_BYTES)
            || (matches!(atom, AtomView::Num(_)) && atom.get_byte_size() > MAX_LEAF_BYTES)
        {
            return self.hole(atom, path);
        }
        self.nodes -= 1;
        match atom {
            AtomView::Num(_) => {
                if self.opts.precision.is_some_and(|p| p > MAX_LEAF_BYTES) {
                    return self.hole(atom, path);
                }
                let text = atom.printer(self.opts.clone()).to_string();
                if text.len() > MAX_LEAF_BYTES {
                    return self.hole(atom, path);
                }
                self.bytes = self.bytes.saturating_sub(text.len());
                let (sign, value) = text
                    .strip_prefix('-')
                    .map_or(("", text.as_str()), |v| ("<mo>−</mo>", v));
                let body = if let Some((n, d)) = value.split_once('/')
                    && n.chars().all(|c| {
                        c.is_ascii_digit() || Some(c) == self.opts.number_thousands_separator
                    })
                    && d.chars().all(|c| {
                        c.is_ascii_digit() || Some(c) == self.opts.number_thousands_separator
                    }) {
                    *parts = Some((row(format!("{sign}{}", token("mn", n))), token("mn", d)));
                    format!("<mfrac>{}{}</mfrac>", token("mn", n), token("mn", d))
                } else if value
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'))
                {
                    token("mn", value)
                } else {
                    // Exotic coefficient domains retain the existing exact notation.
                    token("mtext", value)
                };
                row(format!("{sign}{body}"))
            }
            AtomView::Var(v) => self.name(v.get_symbol()),
            AtomView::Pow(p) => {
                let (base, exp) = p.get_base_exp();
                path.push(0);
                let mut b = self.render(base, path, depth + 1);
                path.pop();
                if let AtomView::Num(n) = exp {
                    if let CoefficientView::Natural(1, 2, 0, 1) = n.get_coeff_view() {
                        return format!("<msqrt>{b}</msqrt>");
                    }
                    if let CoefficientView::Natural(-1, 1, 0, 1) = n.get_coeff_view() {
                        let html = fraction("<mn>1</mn>".to_owned(), b.clone(), parts);
                        if matches!(base, AtomView::Add(_)) {
                            *parts = Some(("<mn>1</mn>".to_owned(), fence(b)));
                        }
                        return html;
                    }
                    if let CoefficientView::Natural(-1, 2, 0, 1) = n.get_coeff_view() {
                        return fraction(
                            "<mn>1</mn>".to_owned(),
                            format!("<msqrt>{b}</msqrt>"),
                            parts,
                        );
                    }
                }
                let positive_integer = matches!(base, AtomView::Num(n)
                    if matches!(n.get_coeff_view(), CoefficientView::Natural(r, 1, 0, 1) if r >= 0));
                if !matches!(base, AtomView::Var(_) | AtomView::Fun(_)) && !positive_integer {
                    b = fence(b);
                }
                path.push(1);
                let e = self.render(exp, path, depth + 1);
                path.pop();
                let real_exponent = if let AtomView::Num(n) = exp {
                    match n.get_coeff_view() {
                        CoefficientView::Natural(_, _, i, _) => i == 0,
                        CoefficientView::Large(_, i) => i.is_zero(),
                        CoefficientView::Float(_, i) => i.is_zero(),
                        _ => false,
                    }
                } else {
                    false
                };
                if real_exponent && let Some(magnitude) = e.strip_prefix("<mrow><mo>−</mo>") {
                    return fraction(
                        "<mn>1</mn>".to_owned(),
                        format!("<msup>{b}<mrow>{magnitude}</msup>"),
                        parts,
                    );
                }
                format!("<msup>{b}{e}</msup>")
            }
            _ => self.sequence(atom, path, depth, 0, self.nested_limit).0,
        }
    }

    fn sequence(
        &mut self,
        atom: AtomView<'_>,
        path: &mut Vec<usize>,
        depth: usize,
        start: usize,
        limit: usize,
    ) -> (String, usize) {
        let (body, end) = self.sequence_inner(atom, path, depth, start, limit);
        (
            if depth > 0 {
                Self::tagged(body, path)
            } else {
                body
            },
            end,
        )
    }

    fn sequence_inner(
        &mut self,
        atom: AtomView<'_>,
        path: &mut Vec<usize>,
        depth: usize,
        start: usize,
        limit: usize,
    ) -> (String, usize) {
        let mut limit = self
            .expansions
            .iter()
            .find(|(p, _)| p == path)
            .map_or(limit, |(_, count)| *count);
        // A compact alternative must retain the ancestors of an expanded part.
        for (p, _) in self.expansions {
            if depth > 0 && p.starts_with(path) && p.len() > path.len() {
                limit = limit.max((p[path.len()] + 1).saturating_sub(start));
            }
        }
        // Responsive abbreviation should not replace a short trailing argument
        // or factor with a wider marker. Explicit root page sizes stay exact.
        let limit = if depth > 0 && derivative_head(atom).is_none() {
            self.compact_tail_limit(atom, start, limit.min(500))
        } else {
            limit.min(500)
        };
        if matches!(atom, AtomView::Mul(_)) {
            return self.product(atom, path, depth, start, limit);
        }
        if let Some((count, symbol)) = derivative_head(atom) {
            if symbol.get_name().len() > MAX_LEAF_BYTES {
                return (self.hole(atom, path), count);
            }
            let AtomView::Fun(f) = atom else {
                unreachable!()
            };
            let head = self.name(symbol);
            let mut orders = String::new();
            let mut arguments = String::new();
            let mut end = start;
            for i in start..count.min(start.saturating_add(limit)) {
                if end > start && (self.nodes == 0 || self.bytes < MAX_LEAF_BYTES) {
                    self.budget_exhausted = true;
                    break;
                }
                let checkpoint = self.checkpoint();
                path.push(i);
                let mut order = self.render(f.get(i), path, depth + 1);
                path.pop();
                path.push(count + 1 + i);
                let mut argument = self.render(f.get(count + 1 + i), path, depth + 1);
                path.pop();
                if depth == 0 && self.budget_exhausted {
                    self.restore(checkpoint);
                    if end > start {
                        break;
                    }
                    path.push(i);
                    order = if self.compact_cost(f.get(i), 4).is_some() {
                        self.render(f.get(i), path, depth + 1)
                    } else {
                        self.hole(f.get(i), path)
                    };
                    path.pop();
                    path.push(count + 1 + i);
                    argument = self.hole(f.get(count + 1 + i), path);
                    path.pop();
                    self.nodes = 0;
                }
                if i > start {
                    orders.push_str("<mo>,</mo>");
                    arguments.push_str("<mo>,</mo>");
                }
                orders.push_str(&order);
                arguments.push_str(&argument);
                end = i + 1;
            }
            if start > 0 {
                orders.insert_str(0, "<mtext>⋯</mtext><mo>,</mo>");
                arguments.insert_str(0, "<mtext>⋯</mtext><mo>,</mo>");
            }
            if end < count {
                orders.push_str("<mo>,</mo><mtext>⋯</mtext>");
                arguments.push_str("<mo>,</mo>");
                arguments.push_str(&self.hole_at(atom, path, end));
            }
            return (
                row(format!(
                    "<msup>{head}{}</msup><mo>&#x2061;</mo>{}",
                    fence(orders),
                    fence(arguments)
                )),
                end,
            );
        }
        let mut body = String::new();
        let expanded_sum = depth > 0
            && matches!(atom, AtomView::Add(_))
            && (self.multiline_sums || self.expansions.iter().any(|(p, _)| p == path));
        let mut expanded_terms = Vec::new();
        let markers_before = self.markers.len();
        let mut arguments = Vec::new();
        let wrap = depth == 0 && matches!(atom, AtomView::Add(_));
        // Reserve the function head before rendering its arguments, so nested
        // long names cannot evade the text budget while unwinding recursion.
        let head = if let AtomView::Fun(f) = atom {
            Some(self.name(f.get_symbol()))
        } else {
            None
        };
        let mut end = start;
        let total = child_count(atom);
        for (i, child) in children(atom).unwrap().enumerate().skip(start).take(limit) {
            // Stop a page before the budget runs out. Nested lists become one
            // navigable omission instead of emitting an unbounded set of markers.
            if end > start && (self.nodes == 0 || self.bytes < MAX_LEAF_BYTES) {
                self.budget_exhausted = true;
                break;
            }
            let checkpoint = self.checkpoint();
            path.push(i);
            let mut fragment = self.render(child, path, depth + 1);
            if depth == 0 && self.budget_exhausted {
                self.restore(checkpoint);
                if end > start {
                    path.pop();
                    break;
                }
                // Even a single oversized item must make progress, but retain
                // one inspectable subtree instead of a partly rendered term.
                fragment = self.hole(child, path);
                self.nodes = 0;
            }
            path.pop();
            let term_start = body.len();
            let separator = match atom {
                AtomView::Add(_) if wrap => "<mo form=\"infix\">+</mo>".to_owned(),
                AtomView::Add(_) => "<mo>+</mo>".to_owned(),
                _ => "<mo>,</mo>".to_owned(),
            };
            if wrap {
                body.push_str("<mrow style=\"flex:0 0 auto\">");
            }
            if i > start {
                if matches!(atom, AtomView::Add(_))
                    && let Some(offset) = leading_minus(&fragment)
                {
                    body.push_str(if wrap {
                        "<mo form=\"infix\">−</mo>"
                    } else {
                        "<mo>−</mo>"
                    });
                    fragment.replace_range(offset..offset + "<mo>−</mo>".len(), "");
                } else if !body.ends_with("<mo>−</mo>") {
                    body.push_str(&separator);
                }
            }
            body.push_str(&fragment);
            if head.is_some() {
                arguments.push(fragment);
            }
            if wrap {
                body.push_str("</mrow>");
            }
            if expanded_sum {
                expanded_terms.push(body[term_start..].to_owned());
            }
            end = i + 1;
        }
        if start > 0 {
            body.insert_str(0, "<mtext>⋯</mtext><mspace width=\"0.5em\"/>");
            if head.is_some() {
                arguments.insert(0, "<mtext>⋯</mtext>".to_owned());
            }
        }
        let nested_markers = self.markers.len() > markers_before;
        if end < total {
            // Root omissions also open a popup, so ordinary output needs no
            // global pager. The inspection view still offers Previous/Next.
            if !matches!(atom, AtomView::Add(_)) {
                body.push_str("<mo>,</mo>");
            }
            let omission = self.hole_at(atom, path, end);
            body.push_str(&omission);
            if expanded_sum {
                expanded_terms.push(omission.clone());
            }
            if head.is_some() {
                arguments.push(omission);
            }
        }
        let columns = self
            .nested_limit
            .clamp(2, if self.multiline_sums { 6 } else { 8 });
        if expanded_sum && expanded_terms.len() > columns {
            body = "<mtable columnalign=\"left\" rowspacing=\"0.35em\">".to_owned();
            for terms in expanded_terms.chunks(columns) {
                body.push_str(&format!("<mtr><mtd>{}</mtd></mtr>", row(terms.concat())));
            }
            body.push_str("</mtable>");
        }
        if let Some(head) = head {
            // Keep sibling arguments visible when one contains an omission.
            // Native MathML table rows preserve math layout inside each argument.
            if arguments.len() > 1 && (nested_markers || (end < total && arguments.len() > 3)) {
                let last = arguments.len() - 1;
                body = "<mtable columnalign=\"left\" rowspacing=\"0.45em\">".to_owned();
                for (index, argument) in arguments.into_iter().enumerate() {
                    let comma = if index < last { "<mo>,</mo>" } else { "" };
                    body.push_str(&format!("<mtr><mtd>{argument}{comma}</mtd></mtr>"));
                }
                body.push_str("</mtable>");
            }
            body = format!("{}<mo>&#x2061;</mo>{}", head, fence(body));
        }
        let body = if wrap {
            format!(
                "<mrow style=\"display:inline-flex;flex-wrap:wrap;align-items:baseline;max-width:min(650px,80vw);row-gap:.5em\">{body}</mrow>"
            )
        } else {
            row(body)
        };
        (body, end)
    }

    fn product(
        &mut self,
        atom: AtomView<'_>,
        path: &mut Vec<usize>,
        depth: usize,
        start: usize,
        limit: usize,
    ) -> (String, usize) {
        // Normal inline products retain implicit multiplication. Pages and
        // explicitly expanded products can span lines; give every subsequent
        // factor a visible operator so a line break cannot look like a sum.
        let expanded = self.expansions.iter().any(|(p, _)| p == path);
        let multiline = (depth == 0 && child_count(atom) > 8) || expanded;
        let previous_nested_limit = self.nested_limit;
        if expanded {
            // Reveal factors before spending the entire page budget inside the
            // first few sums. Detailed subexpressions remain independently
            // inspectable. All ordinary traversal limits still apply.
            let count = child_count(atom).saturating_sub(start).min(limit).max(1);
            self.nested_limit = self.nested_limit.min((self.nodes / count / 8).max(1));
        }
        let mut numerator = Vec::new();
        let mut denominator = Vec::new();
        let mut negative = false;
        let mut end = start;
        for (i, child) in children(atom).unwrap().enumerate().skip(start).take(limit) {
            if end > start && (self.nodes == 0 || self.bytes < MAX_LEAF_BYTES) {
                self.budget_exhausted = true;
                break;
            }
            let checkpoint = self.checkpoint();
            path.push(i);
            let mut parts = None;
            let previous_multiline_sums = self.multiline_sums;
            self.multiline_sums |= multiline;
            let mut html = self.render_factor(child, path, depth + 1, &mut parts);
            self.multiline_sums = previous_multiline_sums;
            if depth == 0 && self.budget_exhausted {
                self.restore(checkpoint);
                if end > start {
                    path.pop();
                    break;
                }
                html = self.hole(child, path);
                parts = None;
                self.nodes = 0;
            }
            path.pop();
            if let Some((n, d)) = parts {
                let n = product_sign(n, &mut negative);
                if n != "<mn>1</mn>" && n != "<mrow><mn>1</mn></mrow>" {
                    numerator.push(n);
                }
                denominator.push(d);
            } else {
                if !complex_coefficient(child) {
                    html = product_sign(html, &mut negative);
                }
                if matches!(child, AtomView::Add(_)) || complex_coefficient(child) {
                    html = fence(html);
                }
                if html != "<mrow><mn>1</mn></mrow>" {
                    numerator.push(html);
                }
            }
            end = i + 1;
        }
        self.nested_limit = previous_nested_limit;
        let mut omission = (end < child_count(atom)).then(|| self.hole_at(atom, path, end));
        let join = |factors: Vec<String>, tail: Option<String>| {
            let mut result = String::new();
            let mut rows = Vec::new();
            let mut row_cost = 0;
            let mut previous_numeric = false;
            for (i, factor) in factors.iter().enumerate() {
                if multiline {
                    let cost = visible_cost(factor) + usize::from(i > 0);
                    if !result.is_empty() && row_cost + cost > PRODUCT_ROW_COST {
                        rows.push(std::mem::take(&mut result));
                        row_cost = 0;
                    }
                    row_cost += cost;
                    result.push_str("<mrow data-symbolica-factor=\"\">");
                }
                let mut leading = factor.as_str();
                while let Some(rest) = leading
                    .strip_prefix("<mrow>")
                    .or_else(|| leading.strip_prefix("<msup>"))
                {
                    leading = rest;
                }
                let numeric = leading.starts_with("<mn>");
                if i > 0 && !result.ends_with("<mo>−</mo>") {
                    if !multiline
                        && self.opts.multiplication_operator == '·'
                        && !(previous_numeric && numeric)
                    {
                        result.push_str("<mo>&#x2062;</mo><mspace width=\"0.167em\"/>");
                    } else {
                        result
                            .push_str(&token("mo", &self.opts.multiplication_operator.to_string()));
                    }
                }
                result.push_str(factor);
                if multiline {
                    result.push_str("</mrow>");
                }
                previous_numeric = numeric;
            }
            if result.is_empty() {
                result.push_str("<mn>1</mn>");
            }
            if multiline {
                rows.push(result);
                if let Some(tail) = tail {
                    rows.push(tail);
                }
                let rows = rows
                    .into_iter()
                    .map(|r| format!("<mtr><mtd>{}</mtd></mtr>", row(r)))
                    .collect::<String>();
                format!(
                    "<mtable data-symbolica-product=\"\" columnalign=\"left\" rowspacing=\"0.5em\">{rows}</mtable>"
                )
            } else {
                row(result)
            }
        };
        let n = join(
            numerator,
            if multiline && denominator.is_empty() {
                omission.take()
            } else {
                None
            },
        );
        let mut body = if denominator.is_empty() {
            n
        } else {
            format!("<mfrac>{n}{}</mfrac>", join(denominator, None))
        };
        if negative {
            body.insert_str(0, "<mo>−</mo>");
        }
        if start > 0 {
            body.insert_str(0, "<mtext>⋯</mtext><mspace width=\"0.5em\"/>");
        }
        if let Some(omission) = omission {
            body.push_str(&omission);
        }
        (row(body), end)
    }
}

/// Render one page, retaining paths to omitted nested expressions. Rendering
/// and output are bounded; locating a page scans the preceding children.
pub(crate) fn render_page(
    atom: AtomView<'_>,
    options: &PrintOptions,
    start: usize,
    limit: usize,
) -> MathMlPage {
    render_layout(atom, options, start, limit, 0, &[])
}

pub(crate) fn render_layout(
    atom: AtomView<'_>,
    options: &PrintOptions,
    start: usize,
    limit: usize,
    layout: usize,
    expansions: &[(Vec<usize>, usize)],
) -> MathMlPage {
    let (nested_limit, depth_limit) = LAYOUTS[layout];
    let opts = PrintOptions {
        color_mode: ColorMode::Never,
        color_builtin_symbols: false,
        num_exp_as_superscript: false,
        max_line_length: None,
        max_terms: None,
        ..options.clone()
    };
    let mut renderer = Renderer {
        opts: &opts,
        nodes: MAX_NODES,
        bytes: MAX_TEXT_BYTES,
        budget_exhausted: false,
        nested_limit,
        depth_limit: depth_limit.min(MAX_DEPTH),
        expansions,
        multiline_sums: false,
        markers: vec![],
    };
    let sequence = matches!(atom, AtomView::Add(_) | AtomView::Mul(_) | AtomView::Fun(_));
    let total = if sequence { item_count(atom).max(1) } else { 1 };
    let start = start.min(total - 1);
    let (body, end) = if sequence
        && child_count(atom) > 0
        && !atom
            .get_symbol()
            .is_some_and(|s| s.get_name().len() > MAX_LEAF_BYTES)
    {
        renderer.sequence(atom, &mut vec![], 0, start, limit.max(1))
    } else {
        (renderer.render(atom, &mut vec![], 0), 1)
    };
    MathMlPage {
        html: format!(
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\" style=\"max-width:100%;overflow-x:auto;font-family:'STIX Two Math',math;font-size:21px;font-synthesis:none\">{body}</math>"
        ),
        start,
        end,
        total,
        markers: renderer.markers,
    }
}
