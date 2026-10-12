//! Tiny TeX → unicode converter for inline math. It is not a TeX engine; it makes common
//! formulas (`\alpha^2 + \sqrt{x} \le \infty`) readable in a terminal.

const SYMBOLS: &[(&str, &str)] = &[
    ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ε"),
    ("varepsilon", "ε"), ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("iota", "ι"),
    ("kappa", "κ"), ("lambda", "λ"), ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"), ("pi", "π"),
    ("rho", "ρ"), ("sigma", "σ"), ("tau", "τ"), ("upsilon", "υ"), ("phi", "φ"),
    ("varphi", "φ"), ("chi", "χ"), ("psi", "ψ"), ("omega", "ω"), ("Gamma", "Γ"),
    ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"),
    ("Sigma", "Σ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"), ("infty", "∞"),
    ("le", "≤"), ("leq", "≤"), ("ge", "≥"), ("geq", "≥"), ("ne", "≠"), ("neq", "≠"),
    ("approx", "≈"), ("equiv", "≡"), ("sim", "∼"), ("pm", "±"), ("mp", "∓"),
    ("times", "×"), ("div", "÷"), ("cdot", "·"), ("ldots", "…"), ("cdots", "⋯"),
    ("dots", "…"), ("sum", "∑"), ("prod", "∏"), ("int", "∫"), ("oint", "∮"),
    ("partial", "∂"), ("nabla", "∇"), ("sqrt", "√"), ("in", "∈"), ("notin", "∉"),
    ("subset", "⊂"), ("subseteq", "⊆"), ("supset", "⊃"), ("cup", "∪"), ("cap", "∩"),
    ("emptyset", "∅"), ("forall", "∀"), ("exists", "∃"), ("neg", "¬"), ("land", "∧"),
    ("lor", "∨"), ("to", "→"), ("rightarrow", "→"), ("leftarrow", "←"),
    ("Rightarrow", "⇒"), ("Leftarrow", "⇐"), ("leftrightarrow", "↔"),
    ("Leftrightarrow", "⇔"), ("mapsto", "↦"), ("implies", "⟹"), ("iff", "⟺"),
    ("circ", "∘"), ("degree", "°"), ("prime", "′"), ("hbar", "ℏ"), ("ell", "ℓ"),
    ("Re", "ℜ"), ("Im", "ℑ"), ("aleph", "ℵ"), ("mathbb{R}", "ℝ"), ("mathbb{N}", "ℕ"),
    ("mathbb{Z}", "ℤ"), ("mathbb{Q}", "ℚ"), ("mathbb{C}", "ℂ"), ("langle", "⟨"),
    ("rangle", "⟩"), ("lfloor", "⌊"), ("rfloor", "⌋"), ("lceil", "⌈"), ("rceil", "⌉"),
    ("quad", "  "), ("qquad", "    "), (",", " "), (";", " "), ("!", ""), ("{", "{"),
    ("}", "}"), ("\\", "\n"), ("left", ""), ("right", ""), ("big", ""), ("Big", ""),
];

fn sup(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶',
        '7' => '⁷', '8' => '⁸', '9' => '⁹', '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽',
        ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ', 'x' => 'ˣ', 'y' => 'ʸ', 'a' => 'ᵃ', 'b' => 'ᵇ',
        'c' => 'ᶜ', 'd' => 'ᵈ', 'e' => 'ᵉ', 'k' => 'ᵏ', 'm' => 'ᵐ', 'T' => 'ᵀ', 't' => 'ᵗ',
        _ => return None,
    })
}

fn sub(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆',
        '7' => '₇', '8' => '₈', '9' => '₉', '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍',
        ')' => '₎', 'a' => 'ₐ', 'e' => 'ₑ', 'i' => 'ᵢ', 'j' => 'ⱼ', 'k' => 'ₖ', 'n' => 'ₙ',
        'o' => 'ₒ', 'x' => 'ₓ', 'm' => 'ₘ', 't' => 'ₜ',
        _ => return None,
    })
}

/// Read a `{group}` or single char starting at `i`; returns (content, next index).
fn group(chars: &[char], i: usize) -> (String, usize) {
    if i >= chars.len() {
        return (String::new(), i);
    }
    if chars[i] == '{' {
        let mut depth = 0;
        let mut j = i;
        while j < chars.len() {
            match chars[j] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return (chars[i + 1..j].iter().collect(), j + 1);
                    }
                }
                _ => {}
            }
            j += 1;
        }
        (chars[i + 1..].iter().collect(), chars.len())
    } else if chars[i] == '\\' {
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_ascii_alphabetic() {
            j += 1;
        }
        (chars[i..j.max(i + 2).min(chars.len())].iter().collect(), j.max(i + 2).min(chars.len()))
    } else {
        (chars[i].to_string(), i + 1)
    }
}

fn script(s: &str, f: fn(char) -> Option<char>, marker: char) -> String {
    let conv = to_unicode(s);
    let mapped: Option<String> = conv.chars().map(f).collect();
    match mapped {
        Some(m) => m,
        None if conv.chars().count() == 1 => format!("{marker}{conv}"),
        None => format!("{marker}({conv})"),
    }
}

pub fn to_unicode(tex: &str) -> String {
    let chars: Vec<char> = tex.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                // \mathbb{X} style first
                let rest: String = chars[i + 1..].iter().collect();
                if let Some((name, sym)) = SYMBOLS
                    .iter()
                    .filter(|(n, _)| rest.starts_with(n))
                    .filter(|(n, _)| {
                        // whole-word match for alphabetic commands
                        let after = rest[n.len()..].chars().next();
                        !n.chars().all(|c| c.is_ascii_alphabetic())
                            || !after.is_some_and(|a| a.is_ascii_alphabetic())
                    })
                    .max_by_key(|(n, _)| n.len())
                {
                    i += 1 + name.chars().count();
                    if *name == "sqrt" {
                        let (g, ni) = group(&chars, i);
                        out.push('√');
                        let inner = to_unicode(&g);
                        if inner.chars().count() > 1 {
                            out.push_str(&format!("({inner})"));
                        } else {
                            out.push_str(&inner);
                        }
                        i = ni;
                    } else {
                        out.push_str(sym);
                    }
                    continue;
                }
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_alphabetic() {
                    j += 1;
                }
                let cmd: String = chars[i + 1..j].iter().collect();
                i = j;
                match cmd.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let (a, ni) = group(&chars, i);
                        let (b, ni) = group(&chars, ni);
                        i = ni;
                        let (a, b) = (to_unicode(&a), to_unicode(&b));
                        let wrap = |s: String| {
                            if s.chars().count() > 1 { format!("({s})") } else { s }
                        };
                        out.push_str(&format!("{}/{}", wrap(a), wrap(b)));
                    }
                    "text" | "mathrm" | "mathbf" | "mathit" | "operatorname" | "textbf"
                    | "mathsf" | "mathcal" | "boldsymbol" => {
                        let (a, ni) = group(&chars, i);
                        i = ni;
                        out.push_str(&to_unicode(&a));
                    }
                    "" => {
                        if i < chars.len() {
                            out.push(chars[i]);
                            i += 1;
                        }
                    }
                    other => out.push_str(other),
                }
            }
            '^' => {
                let (g, ni) = group(&chars, i + 1);
                out.push_str(&script(&g, sup, '^'));
                i = ni;
            }
            '_' => {
                let (g, ni) = group(&chars, i + 1);
                out.push_str(&script(&g, sub, '_'));
                i = ni;
            }
            '{' | '}' => i += 1,
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::to_unicode;

    #[test]
    fn converts_common_tex() {
        assert_eq!(to_unicode(r"\alpha^2 + \beta_1"), "α² + β₁");
        assert_eq!(to_unicode(r"\frac{a}{b} \le \infty"), "a/b ≤ ∞");
        assert_eq!(to_unicode(r"\sqrt{x+1}"), "√(x+1)");
        assert_eq!(to_unicode(r"e^{i\pi}"), "e^(iπ)");
    }
}
