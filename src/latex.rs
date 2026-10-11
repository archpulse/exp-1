//! LaTeX math to Unicode text.
//!
//! This is a converter, not a TeX engine: it maps symbols, fractions, roots,
//! sub/superscripts and font variants onto characters a terminal can show.
//! Anything it does not understand is emitted verbatim rather than dropped, so
//! unfamiliar input degrades instead of turning into garbage.
//!
//! Single pass, one hash lookup per command: rendering a long answer costs
//! microseconds and never blocks the UI.

/// Greek lowercase, then uppercase.
const GREEK_LOWER: &[(&str, char)] = &[
    ("alpha", 'α'),
    ("beta", 'β'),
    ("gamma", 'γ'),
    ("delta", 'δ'),
    ("epsilon", 'ε'),
    ("zeta", 'ζ'),
    ("eta", 'η'),
    ("theta", 'θ'),
    ("iota", 'ι'),
    ("kappa", 'κ'),
    ("lambda", 'λ'),
    ("mu", 'μ'),
    ("nu", 'ν'),
    ("xi", 'ξ'),
    ("pi", 'π'),
    ("rho", 'ρ'),
    ("sigma", 'σ'),
    ("tau", 'τ'),
    ("upsilon", 'υ'),
    ("phi", 'φ'),
    ("chi", 'χ'),
    ("psi", 'ψ'),
    ("omega", 'ω'),
    ("varpi", 'ϖ'),
];

const GREEK_UPPER: &[(&str, char)] = &[
    ("Gamma", 'Γ'),
    ("Delta", 'Δ'),
    ("Theta", 'Θ'),
    ("Lambda", 'Λ'),
    ("Xi", 'Ξ'),
    ("Pi", 'Π'),
    ("Sigma", 'Σ'),
    ("Upsilon", 'Υ'),
    ("Phi", 'Φ'),
    ("Psi", 'Ψ'),
    ("Omega", 'Ω'),
    ("Alpha", 'Α'),
    ("Beta", 'Β'),
    ("Epsilon", 'Ε'),
    ("Zeta", 'Ζ'),
    ("Eta", 'Η'),
    ("Iota", 'Ι'),
    ("Kappa", 'Κ'),
];

/// Single-character replacements: relations, operators, arrows, delimiters.
const SYMBOLS: &[(&str, char)] = &[
    ("times", '×'),
    ("div", '÷'),
    ("pm", '±'),
    ("mp", '∓'),
    ("cdot", '⋅'),
    ("cdots", '⋯'),
    ("ldots", '…'),
    ("vdots", '⋮'),
    ("ddots", '⋱'),
    ("leq", '≤'),
    ("le", '≤'),
    ("geq", '≥'),
    ("ge", '≥'),
    ("neq", '≠'),
    ("ne", '≠'),
    ("approx", '≈'),
    ("equiv", '≡'),
    ("simeq", '≃'),
    ("cong", '≅'),
    ("sim", '∼'),
    ("propto", '∝'),
    ("infty", '∞'),
    ("partial", '∂'),
    ("nabla", '∇'),
    ("in", '∈'),
    ("notin", '∉'),
    ("ni", '∋'),
    ("subset", '⊂'),
    ("supset", '⊃'),
    ("subseteq", '⊆'),
    ("supseteq", '⊇'),
    ("cup", '∪'),
    ("cap", '∩'),
    ("setminus", '∖'),
    ("forall", '∀'),
    ("exists", '∃'),
    ("nexists", '∄'),
    ("emptyset", '∅'),
    ("varnothing", '∅'),
    ("neg", '¬'),
    ("lnot", '¬'),
    ("land", '∧'),
    ("wedge", '∧'),
    ("lor", '∨'),
    ("vee", '∨'),
    ("oplus", '⊕'),
    ("otimes", '⊗'),
    ("perp", '⊥'),
    ("parallel", '∥'),
    ("mid", '∣'),
    ("angle", '∠'),
    ("therefore", '∴'),
    ("because", '∵'),
    ("aleph", 'ℵ'),
    ("hbar", 'ℏ'),
    ("ell", 'ℓ'),
    ("Re", 'ℜ'),
    ("Im", 'ℑ'),
    ("wp", '℘'),
    ("prime", '′'),
    ("to", '→'),
    ("rightarrow", '→'),
    ("leftarrow", '←'),
    ("uparrow", '↑'),
    ("downarrow", '↓'),
    ("leftrightarrow", '↔'),
    ("Rightarrow", '⇒'),
    ("Leftarrow", '⇐'),
    ("Leftrightarrow", '⇔'),
    ("iff", '⇔'),
    ("mapsto", '↦'),
    ("longmapsto", '⟼'),
    ("longrightarrow", '⟶'),
    ("longleftarrow", '⟵'),
    ("sum", '∑'),
    ("prod", '∏'),
    ("coprod", '∐'),
    ("int", '∫'),
    ("iint", '∬'),
    ("iiint", '∭'),
    ("oint", '∮'),
    ("sqrt", '√'),
    ("langle", '⟨'),
    ("rangle", '⟩'),
    ("lceil", '⌈'),
    ("rceil", '⌉'),
    ("lfloor", '⌊'),
    ("rfloor", '⌋'),
    ("checkmark", '✓'),
    ("dagger", '†'),
];

/// Two-argument operators rendered inline with Unicode.
const BINARY: &[(&str, char)] = &[
    ("frac", '⁄'),
    ("dfrac", '⁄'),
    ("tfrac", '⁄'),
    ("cfrac", '⁄'),
];

const SUPERSCRIPT: &[(char, char)] = &[
    ('0', '⁰'),
    ('1', '¹'),
    ('2', '²'),
    ('3', '³'),
    ('4', '⁴'),
    ('5', '⁵'),
    ('6', '⁶'),
    ('7', '⁷'),
    ('8', '⁸'),
    ('9', '⁹'),
    ('+', '⁺'),
    ('-', '⁻'),
    ('=', '⁼'),
    ('(', '⁽'),
    (')', '⁾'),
    ('n', 'ⁿ'),
    ('i', 'ⁱ'),
    ('a', 'ᵃ'),
    ('b', 'ᵇ'),
    ('c', 'ᶜ'),
    ('d', 'ᵈ'),
    ('e', 'ᵉ'),
    ('k', 'ᵏ'),
    ('m', 'ᵐ'),
    ('p', 'ᵖ'),
    ('t', 'ᵗ'),
];

const SUBSCRIPT: &[(char, char)] = &[
    ('0', '₀'),
    ('1', '₁'),
    ('2', '₂'),
    ('3', '₃'),
    ('4', '₄'),
    ('5', '₅'),
    ('6', '₆'),
    ('7', '₇'),
    ('8', '₈'),
    ('9', '₉'),
    ('+', '₊'),
    ('-', '₋'),
    ('=', '₌'),
    ('(', '₍'),
    (')', '₎'),
    ('a', 'ₐ'),
    ('e', 'ₑ'),
    ('i', 'ᵢ'),
    ('j', 'ⱼ'),
    ('n', 'ₙ'),
    ('s', 'ₛ'),
    ('x', 'ₓ'),
];


/// Combining marks for the common accent macros.
const ACCENTS: &[(&str, char)] = &[
    ("bar", '\u{0304}'),
    ("overline", '\u{0305}'),
    ("hat", '\u{0302}'),
    ("widehat", '\u{0302}'),
    ("tilde", '\u{0303}'),
    ("widetilde", '\u{0303}'),
    ("dot", '\u{0307}'),
    ("ddot", '\u{0308}'),
    ("check", '\u{030C}'),
    ("vec", '\u{20D7}'),
];

/// Operators that stay as their upright name.
const NAMES: &[(&str, &str)] = &[
    ("log", "log"),
    ("ln", "ln"),
    ("exp", "exp"),
    ("sin", "sin"),
    ("cos", "cos"),
    ("tan", "tan"),
    ("cot", "cot"),
    ("sec", "sec"),
    ("csc", "csc"),
    ("sinh", "sinh"),
    ("cosh", "cosh"),
    ("tanh", "tanh"),
    ("arcsin", "arcsin"),
    ("arccos", "arccos"),
    ("arctan", "arctan"),
    ("min", "min"),
    ("max", "max"),
    ("sup", "sup"),
    ("inf", "inf"),
    ("lim", "lim"),
    ("limsup", "lim sup"),
    ("liminf", "lim inf"),
    ("det", "det"),
    ("gcd", "gcd"),
    ("deg", "deg"),
    ("dim", "dim"),
    ("ker", "ker"),
    ("arg", "arg"),
];

const CALLIGRAPHIC: &[(char, char)] = &[
    ('L', 'ℒ'),
    ('M', 'ℳ'),
    ('N', 'ℕ'),
    ('O', 'ℴ'),
    ('P', '℘'),
    ('R', 'ℛ'),
    ('S', '𝒮'),
    ('T', '𝒯'),
    ('U', '𝒰'),
    ('V', '𝒱'),
    ('W', '𝒲'),
    ('H', 'ℋ'),
];

fn lookup(table: &[(&str, char)], name: &str) -> Option<char> {
    table
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
}

fn in_blackboard(ch: char) -> Option<char> {
    match ch {
        'C' => Some('ℂ'),
        'H' => Some('ℍ'),
        'N' => Some('ℕ'),
        'P' => Some('ℙ'),
        'Q' => Some('ℚ'),
        'R' => Some('ℝ'),
        _ => None,
    }
}

fn in_calligraphic(ch: char) -> Option<char> {
    CALLIGRAPHIC
        .iter()
        .find(|(key, _)| *key == ch)
        .map(|&(_, value)| value)
}

fn to_superscript(text: &str) -> String {
    text.chars()
        .map(|ch| {
            SUPERSCRIPT
                .iter()
                .find(|(key, _)| *key == ch)
                .map(|&(_, value)| value)
                .unwrap_or(ch)
        })
        .collect()
}

fn to_subscript(text: &str) -> String {
    text.chars()
        .map(|ch| {
            SUBSCRIPT
                .iter()
                .find(|(key, _)| *key == ch)
                .map(|&(_, value)| value)
                .unwrap_or(ch)
        })
        .collect()
}

/// Reads a `{...}` group or a single token, returning its contents.
fn read_group(chars: &[char], start: usize) -> (String, usize) {
    if start >= chars.len() {
        return (String::new(), start);
    }
    if chars[start] != '{' {
        let ch = chars[start];
        return (ch.to_string(), start + 1);
    }
    let mut depth = 0usize;
    let mut out = String::new();
    let mut index = start;
    while index < chars.len() {
        let ch = chars[index];
        if ch == '{' {
            depth += 1;
            if depth == 1 {
                index += 1;
                continue;
            }
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                index += 1;
                break;
            }
        }
        out.push(ch);
        index += 1;
    }
    (out, index)
}

/// Converts a LaTeX math expression into Unicode text.
pub fn render(src: &str) -> String {
    let chars: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut index = 0usize;

    while index < chars.len() {
        let ch = chars[index];

        // Escape: \| \{ \} \\ \$ \& \% \# \_ \{ \} \  -> literal
        if ch == '\\' && index + 1 < chars.len() {
            let next = chars[index + 1];
            if !next.is_ascii_alphabetic() {
                out.push(match next {
                    '|' => '‖',
                    '\\' => '\\',
                    _ => next,
                });
                index += 2;
                continue;
            }

            // A command name: letters only.
            let mut end = index + 1;
            while end < chars.len() && chars[end].is_ascii_alphabetic() {
                end += 1;
            }
            let name: String = chars[index + 1..end].iter().collect();
            index = end;

            match name.as_str() {
                "frac" | "dfrac" | "tfrac" | "cfrac" => {
                    let (numerator, next) = read_group(&chars, index);
                    let (denominator, after) = read_group(&chars, next);
                    index = after;
                    let numerator = render(&numerator);
                    let denominator = render(&denominator);
                    if !numerator.is_empty() && !denominator.is_empty() {
                        out.push_str(&numerator);
                        out.push('⁄');
                        out.push_str(&denominator);
                    } else {
                        out.push_str(&numerator);
                        out.push_str(&denominator);
                    }
                }
                "sqrt" => {
                    out.push('√');
                    let (_, next) = read_optional_args(&chars, index);
                    let (body, after) = read_group(&chars, next);
                    index = after;
                    let body = render(&body);
                    if body.is_empty() {
                        out.pop();
                    } else {
                        out.push_str(&body);
                    }
                }
                "binom" => {
                    let (top, next) = read_group(&chars, index);
                    let (bottom, after) = read_group(&chars, next);
                    index = after;
                    let top = render(&top);
                    let bottom = render(&bottom);
                    out.push('(');
                    out.push_str(&top);
                    out.push(' ');
                    out.push_str(&bottom);
                    out.push(')');
                }
                "text" | "mathrm" | "textrm" | "mbox" | "operatorname" | "mathsf" | "mathit" => {
                    let (body, next) = read_group(&chars, index);
                    index = next;
                    out.push_str(&body);
                }
                "mathbb" | "Bbb" => {
                    let (body, next) = read_group(&chars, index);
                    index = next;
                    let mapped: String = body
                        .chars()
                        .map(|c| in_blackboard(c).unwrap_or(c))
                        .collect();
                    out.push_str(&mapped);
                }
                "mathcal" => {
                    let (body, next) = read_group(&chars, index);
                    index = next;
                    let mapped: String = body
                        .chars()
                        .map(|c| in_calligraphic(c).unwrap_or(c))
                        .collect();
                    out.push_str(&mapped);
                }
                "mathbf" | "bf" | "textbf" | "mathitx" => {
                    let (body, next) = read_group(&chars, index);
                    index = next;
                    out.push_str(&body);
                }
                "overline" | "bar" | "hat" | "widehat" | "tilde" | "widetilde" | "dot"
                | "ddot" | "check" | "vec" => {
                    let (body, next) = read_group(&chars, index);
                    index = next;
                    let rendered = render(&body);
                    let mark = ACCENTS
                        .iter()
                        .find(|(key, _)| *key == name)
                        .map(|(_, mark)| *mark)
                        .unwrap_or('\u{0304}');
                    let mut chars_iter = rendered.chars();
                    match chars_iter.next() {
                        Some(first) => {
                            out.push(first);
                            out.push(mark);
                            out.push_str(chars_iter.as_str());
                        }
                        None => out.push_str(&rendered),
                    }
                }
                "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "displaystyle"
                | "limits" | "nolimits" | "!" => {
                    // Size hints carry no meaning in plain text.
                }
                "quad" => out.push_str("  "),
                "qquad" => out.push_str("    "),
                "," | ";" | ":" | " " => out.push(' '),
                "newline" => out.push('\n'),
                _ => {
                    if let Some((_, word)) = NAMES.iter().find(|(key, _)| *key == name) {
                        out.push_str(word);
                    } else if let Some(symbol) = lookup(&GREEK_LOWER, &name) {
                        out.push(symbol);
                    } else if let Some(symbol) = lookup(&GREEK_UPPER, &name) {
                        out.push(symbol);
                    } else if let Some(symbol) = lookup(&SYMBOLS, &name) {
                        out.push(symbol);
                    } else if lookup(&BINARY, &name).is_some() {
                        // Operators we cannot place without both operands.
                        out.push_str(&name);
                    } else {
                        // Unknown command: keep it verbatim, braces included, so
                        // unfamiliar input stays readable instead of mangled.
                        out.push('\\');
                        out.push_str(&name);
                        if chars.get(index) == Some(&'{') {
                            let (body, next) = read_group(&chars, index);
                            index = next;
                            out.push('{');
                            out.push_str(&body);
                            out.push('}');
                        }
                    }
                }
            }
            continue;
        }

        // Superscript / subscript.
        if ch == '^' || ch == '_' {
            let (body, next) = read_group(&chars, index + 1);
            index = next;
            let mapped = if ch == '^' {
                to_superscript(&body)
            } else {
                to_subscript(&body)
            };
            out.push_str(&mapped);
            continue;
        }

        // Spacing and delimiter hints.
        match ch {
            '{' | '}' => {}
            _ => out.push(ch),
        }
        index += 1;
    }

    out
}

/// `\sqrt[3]{x}` — the optional index before the radicand.
fn read_optional_args(chars: &[char], start: usize) -> (usize, usize) {
    if start < chars.len() && chars[start] == '[' {
        let mut index = start;
        while index < chars.len() && chars[index] != ']' {
            index += 1;
        }
        let next = (index + 1).min(chars.len());
        return (next, next);
    }
    (start, start)
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn greek_and_operators() {
        assert_eq!(render(r"\alpha + \beta"), "α + β");
        assert_eq!(render(r"\sum_{i=1}^{n} x_i"), "∑ᵢ₌₁ⁿ xᵢ");
        assert_eq!(render(r"a \leq b"), "a ≤ b");
    }

    #[test]
    fn fractions_roots_and_scripts() {
        assert_eq!(render(r"\frac{1}{2}"), "1⁄2");
        assert_eq!(render(r"\sqrt{x^2}"), "√x²");
        assert_eq!(render(r"x^{10}"), "x¹⁰");
    }

    #[test]
    fn font_variants() {
        assert_eq!(render(r"\mathbb{R}"), "ℝ");
        assert_eq!(render(r"\text{hello}"), "hello");
    }

    #[test]
    fn unknown_commands_survive() {
        assert_eq!(render(r"\notacommand{x}"), r"\notacommand{x}");
    }

    #[test]
    fn accents_and_operator_names() {
        assert_eq!(render(r"\bar{x}"), "x\u{304}");
        assert_eq!(render(r"\hat{y}"), "y\u{302}");
        assert_eq!(render(r"\log_2 n"), "log₂ n");
    }

    #[test]
    fn braces_and_escapes() {
        assert_eq!(render(r"\{x\}"), "{x}");
        assert_eq!(render(r"a \\ b"), "a \\ b");
    }
}