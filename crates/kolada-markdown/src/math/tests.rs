use super::*;

fn atom(text: &str, class: Class, font: Font) -> Node {
    Node::Atom { text: text.into(), class, font }
}

fn var(c: &str) -> Node {
    atom(c, Class::Ord, Font::Italic)
}

fn num(n: &str) -> Node {
    atom(n, Class::Ord, Font::Upright)
}

#[test]
fn reads_letters_numbers_and_operators_with_their_classes() {
    assert_eq!(parse("x + 12.5 = y"), [var("x"), atom("+", Class::Bin, Font::Upright), num("12.5"), atom("=", Class::Rel, Font::Upright), var("y")]);
    // Minus is the minus sign, not a hyphen; Greek lowercase is italic,
    // uppercase upright; spaces in the source are not math.
    assert_eq!(parse("a-b"), [var("a"), atom("\u{2212}", Class::Bin, Font::Upright), var("b")]);
    assert_eq!(parse(r"\alpha\Omega \leq \infty"), [var("α"), atom("Ω", Class::Ord, Font::Upright), atom("≤", Class::Rel, Font::Upright), atom("∞", Class::Ord, Font::Upright)]);
    assert_eq!(parse(r"\sin x"), [atom("sin", Class::Op, Font::Upright), var("x")]);
}

#[test]
fn scripts_take_one_token_or_a_group() {
    let sup = |base: Node, s: Node| Node::Scripts { base: Box::new(base), sub: None, sup: Some(Box::new(s)) };
    assert_eq!(parse("x^23"), [sup(var("x"), num("2")), num("3")]);
    assert_eq!(parse("x^{23}"), [sup(var("x"), Node::Group(vec![num("23")]))]);
    // Both, in either order, on one base.
    let both = parse("x_i^2");
    assert_eq!(both, parse("x^2_i"));
    let Node::Scripts { sub: Some(_), sup: Some(_), .. } = &both[0] else { panic!("{both:?}") };
    // A prime is a superscript, and joins the one after it.
    let Node::Scripts { sup: Some(s), .. } = &parse("f'^2")[0] else { panic!() };
    assert_eq!(**s, Node::Group(vec![atom("\u{2032}", Class::Ord, Font::Upright), num("2")]));
}

#[test]
fn reads_fractions_roots_and_big_operators() {
    let Node::Frac { num: n, den, bar: true, .. } = &parse(r"\frac12")[0] else { panic!() };
    assert_eq!((**n).clone(), num("1"));
    assert_eq!((**den).clone(), num("2"));
    let Node::Frac { bar: false, delims: Some(('(', ')')), .. } = &parse(r"\binom{n}{k}")[0] else { panic!() };
    let Node::Frac { style: Some(MathStyle::Display), .. } = &parse(r"\dfrac a b")[0] else { panic!() };
    let Node::Sqrt { index: Some(_), .. } = &parse(r"\sqrt[3]{x}")[0] else { panic!() };
    let Node::Sqrt { index: None, .. } = &parse(r"\sqrt x")[0] else { panic!() };
    let Node::Scripts { base, .. } = &parse(r"\sum_{i=1}^n i")[0] else { panic!() };
    assert_eq!(**base, Node::BigOp { text: "∑".into(), symbol: true, limits: None });
    let Node::Scripts { base, .. } = &parse(r"\int\limits_0^1")[0] else { panic!() };
    assert_eq!(**base, Node::BigOp { text: "∫".into(), symbol: true, limits: Some(true) });
    let Node::BigOp { text, symbol: false, .. } = &parse(r"\lim")[0] else { panic!() };
    assert_eq!(text, "lim");
    // `\over` makes a fraction of what is either side of it in its group.
    let Node::Group(g) = &parse(r"{a \over b}")[0] else { panic!() };
    let Node::Frac { bar: true, .. } = &g[0] else { panic!("{g:?}") };
}

#[test]
fn reads_delimiters_accents_and_text() {
    let Node::Fenced { left: Delim::Char('('), right: Delim::Char(')'), body } = &parse(r"\left( \frac a b \right)")[0] else { panic!() };
    assert_eq!(body.len(), 1);
    let Node::Fenced { left: Delim::Char('{'), right: Delim::None, .. } = &parse(r"\left\{ x \right.")[0] else { panic!() };
    let Node::Big { delim: Delim::Char('['), size: 2, class: Class::Open } = &parse(r"\Bigl[")[0] else { panic!() };
    let Node::Accent { accent: Accent::Hat, .. } = &parse(r"\hat x")[0] else { panic!() };
    assert_eq!(parse(r"\text{if } x"), [Node::Text { text: "if ".into(), font: Font::Upright }, var("x")]);
    assert_eq!(parse(r"\mathbb{R}"), [Node::Group(vec![atom("ℝ", Class::Ord, Font::Upright)])]);
    assert_eq!(parse(r"\mathbf x"), [atom("x", Class::Ord, Font::Bold)]);
    assert_eq!(parse(r"\not="), [atom("≠", Class::Rel, Font::Upright)]);
}

#[test]
fn reads_matrices_cases_and_lines() {
    let Node::Array { rows, left: Delim::Char('('), right: Delim::Char(')'), kind: ArrayKind::Matrix, .. } = &parse(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}")[0] else { panic!() };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.len() == 2));
    let Node::Array { kind: ArrayKind::Cases, left: Delim::Char('{'), rows, .. } = &parse(r"\begin{cases} 1 & x > 0 \\ 0 & \text{else} \\ \end{cases}")[0] else { panic!() };
    assert_eq!(rows.len(), 2, "a last \\\\ starts no row");
    // Lines at the top level, and equations lined up at `&`.
    let Node::Array { rows, kind: ArrayKind::Aligned, cols, .. } = &parse(r"a = b \\ c = d")[0] else { panic!() };
    assert_eq!((rows.len(), cols.as_slice()), (2, [ColAlign::Center].as_slice()));
    let Node::Array { rows, cols, .. } = &parse(r"\begin{aligned} a &= b \\ &= c \end{aligned}")[0] else { panic!() };
    assert_eq!((rows.len(), rows[0].len(), cols.len()), (2, 2, 0));
}

#[test]
fn what_it_cannot_read_stays_in_place() {
    assert_eq!(parse(r"\nosuchthing x"), [Node::Error(r"\nosuchthing".into()), var("x")]);
    // An open brace closes at the end; a stray closing one is nothing.
    assert_eq!(parse("{x"), [Node::Group(vec![var("x")])]);
    assert_eq!(parse("x}"), [var("x")]);
    // Colours are read and left out.
    assert_eq!(parse(r"\color{red} x \textcolor{blue}{y}"), [var("x"), Node::Group(vec![var("y")])]);
    // Nothing nests deeper than the stack can take.
    let deep = "{".repeat(10_000) + "x";
    assert!(!parse(&deep).is_empty());
    let deep = r"\sqrt".repeat(10_000) + "x";
    assert!(!parse(&deep).is_empty());
    // Whatever is typed, it reads: no panics on any prefix of a formula.
    let src = r"\left( \frac{a_1^{2}}{\sqrt[3]{b}} \right) + \begin{bmatrix} 1 & 2 \\ 3 \end{bmatrix} \text{ok}\; x'' \not\in";
    for (i, _) in src.char_indices() {
        parse(&src[..i]);
    }
}


/// Math read as text takes time in step with its size, however deep its
/// scripts nest: a card's worker never stalls on one note.
#[test]
fn math_read_as_text_stays_quick_however_deep() {
    let deep = format!("{}x{}", "x^{".repeat(30), "}".repeat(30));
    let at = std::time::Instant::now();
    let text = crate::math::linear(&deep);
    assert!(at.elapsed() < std::time::Duration::from_millis(50), "{:?}", at.elapsed());
    assert!(text.starts_with('x'));
}
