use super::*;

fn with_eng(f: impl FnOnce(&Eng)) {
    let eng = Eng::new(kolada_text::Engine::new(crate::testing::fonts()));
    f(&eng);
}

fn set(eng: &Eng, src: &str, display: bool) -> Rc<Formula> {
    formula(eng, src, 16.0, display)
}

fn bars(f: &Formula) -> Vec<[f32; 4]> {
    f.items.iter().filter_map(|i| if let Item::Bar(r) = i { Some(*r) } else { None }).collect()
}

fn shapes(f: &Formula) -> Vec<([f32; 4], Polygons)> {
    f.items.iter().filter_map(|i| if let Item::Shape { rect, polygons } = i { Some((*rect, polygons.clone())) } else { None }).collect()
}

#[test]
fn scripts_raise_and_lower_and_take_room() {
    with_eng(|eng| {
        let x = set(eng, "x", false);
        let sup = set(eng, "x^2", false);
        let sub = set(eng, "x_i", false);
        assert!(sup.width > x.width && sub.width > x.width);
        assert!(sup.ascent > x.ascent + 3.0, "the 2 rides above the x: {} vs {}", sup.ascent, x.ascent);
        assert!(sub.descent > 1.5, "the i hangs below the baseline: {}", sub.descent);
        // Both on one base stack, as wide as the wider.
        let both = set(eng, "x_i^2", false);
        assert!(both.width < sup.width + sub.width - x.width + 0.5);
        assert!(both.ascent >= sup.ascent - 0.01 && both.descent >= sub.descent - 0.01);
    });
}

#[test]
fn a_fraction_sits_on_the_axis_with_its_bar_between() {
    with_eng(|eng| {
        let f = set(eng, r"\frac{a}{b}", true);
        let b = bars(&f);
        assert_eq!(b.len(), 1, "one bar");
        let [l, t, r, bot] = b[0];
        let axis = AXIS * 16.0;
        assert!(((t + bot) / 2.0 + axis).abs() < 0.01, "the bar on the axis");
        assert!(l >= 0.0 && r <= f.width);
        assert!(f.ascent > axis + 8.0 && f.descent > 6.0, "{} {}", f.ascent, f.descent);
        // In a line it is smaller than in a block.
        let inline = set(eng, r"\frac{a}{b}", false);
        assert!(inline.ascent + inline.descent < f.ascent + f.descent);
        // A binomial has no bar but its parentheses.
        let binom = set(eng, r"\binom{n}{k}", true);
        assert!(bars(&binom).is_empty());
        assert_eq!(shapes(&binom).len(), 2, "tall parentheses, filled");
    });
}

#[test]
fn roots_and_tall_delimiters_are_filled_shapes_inside_their_boxes() {
    with_eng(|eng| {
        for src in [r"\sqrt{x}", r"\sqrt[3]{\frac{a}{b}}", r"\left( \frac{a}{b} \right)", r"\left\{ \frac{a}{b} \right]", r"\left\langle \frac{a}{b} \middle| c \right\rangle", r"\overbrace{a+b}^{n}", r"\widehat{abc}"] {
            let f = set(eng, src, true);
            let s = shapes(&f);
            assert!(!s.is_empty(), "{src}: drawn from outlines");
            for ([l, t, r, b], polygons) in s {
                assert!(r > l && b > t, "{src}");
                for p in polygons.iter() {
                    assert!(p.len() >= 3);
                    for &(x, y) in p {
                        assert!(x >= -0.5 && x <= r - l + 0.5 && y >= -0.5 && y <= b - t + 0.5, "{src}: ({x}, {y}) outside {}x{}", r - l, b - t);
                    }
                }
                assert!(t >= -f.ascent - 0.5 && b <= f.descent + 0.5, "{src}: inside the formula");
            }
        }
        // A root is as tall as what is under it.
        let low = set(eng, r"\sqrt{x}", true);
        let tall = set(eng, r"\sqrt{\frac{a}{b}}", true);
        assert!(tall.ascent > low.ascent + 6.0);
    });
}

#[test]
fn operators_space_as_tex_spaces_them() {
    with_eng(|eng| {
        let tight = set(eng, "ab", false).width;
        let bin = set(eng, "a+b", false).width - set(eng, "+", false).width;
        let rel = set(eng, "a=b", false).width - set(eng, "=", false).width;
        // Medium space either side of +, thick either side of =.
        assert!((bin - tight - 2.0 * 4.0 / 18.0 * 16.0).abs() < 1.5, "{bin} {tight}");
        assert!((rel - tight - 2.0 * 5.0 / 18.0 * 16.0).abs() < 1.5, "{rel} {tight}");
        // A sign with nothing before it is a sign, not an operation.
        let neg = set(eng, "-a", false).width;
        assert!(neg < set(eng, "-", false).width + set(eng, "a", false).width + 1.0, "{neg}");
        // In scripts, no space: there a + spaces as one in braces does,
        // which is an ordinary symbol.
        let spaced = set(eng, "a+b", false).width - set(eng, "a{+}b", false).width;
        assert!((spaced - 2.0 * 4.0 / 18.0 * 16.0).abs() < 0.01, "{spaced}");
        let script = set(eng, "x^{a+b}", false).width - set(eng, "x^{a{+}b}", false).width;
        assert!(script.abs() < 0.01, "{script}");
    });
}

#[test]
fn big_operators_take_their_limits_above_and_below_in_a_block() {
    with_eng(|eng| {
        let block = set(eng, r"\sum_{i=1}^{n} i", true);
        let inline = set(eng, r"\sum_{i=1}^{n} i", false);
        assert!(block.ascent + block.descent > inline.ascent + inline.descent + 10.0);
        assert!(block.width < inline.width + 4.0, "limits stack rather than trail");
        // An integral keeps its scripts beside it.
        let int = set(eng, r"\int_0^1 x\,dx", true);
        assert!(int.width > set(eng, r"\int x\,dx", true).width + 4.0);
    });
}

#[test]
fn matrices_and_cases_centre_on_the_axis() {
    with_eng(|eng| {
        let m = set(eng, r"\begin{pmatrix} 1 & 2 \\ 3 & 4 \end{pmatrix}", true);
        let axis = AXIS * 16.0;
        assert!(((m.ascent - axis) - (m.descent + axis)).abs() < 1.0, "{} {}", m.ascent, m.descent);
        assert_eq!(shapes(&m).len(), 2);
        let c = set(eng, r"f(x) = \begin{cases} 1 & x > 0 \\ 0 & \text{otherwise} \end{cases}", true);
        assert_eq!(shapes(&c).len(), 1, "one brace, on the left");
        // Lines of a block, centred: taller than one.
        let lines = set(eng, r"a = b \\ c = d", true);
        assert!(lines.ascent + lines.descent > 2.0 * 16.0);
    });
}

#[test]
fn what_it_cannot_read_shows_in_the_error_ink_and_the_rest_still_sets() {
    with_eng(|eng| {
        let f = set(eng, r"x + \nosuch y", false);
        assert!(f.has_errors());
        assert!(!set(eng, r"x + y", false).has_errors());
        assert!(f.width > set(eng, "x+y", false).width);
        // Nothing panics on any prefix of a formula as it is typed.
        let src = r"\left( \sqrt[3]{\frac{a_1^{2}}{b}} \right) + \begin{bmatrix} 1 & 2 \\ 3 \end{bmatrix} \overbrace{x}^{n} \hat{x}' \boxed{e}";
        for (i, _) in src.char_indices() {
            set(eng, &src[..i], true);
            set(eng, &src[..i], false);
        }
    });
}

#[test]
fn a_formula_is_set_once_while_it_shows() {
    with_eng(|eng| {
        let a = set(eng, r"\frac{1}{2}", true);
        let b = set(eng, r"\frac{1}{2}", true);
        assert!(Rc::ptr_eq(&a, &b));
        assert!(!Rc::ptr_eq(&a, &set(eng, r"\frac{1}{2}", false)), "a line's is another");
        // Drawn: glyphs as texts, the bar as a fill, in the ink asked.
        let mut scene = Scene::default();
        a.draw(10.0, 50.0, Ink::Text, 1.0, &mut scene);
        assert_eq!(scene.texts.len(), 2);
        assert!(scene.texts.iter().all(|t| t.ink == Ink::Text));
        assert_eq!(scene.fills.len(), 1);
        let mut scene = Scene::default();
        set(eng, r"\sqrt{2}", true).draw(0.0, 20.0, Ink::Muted, 0.5, &mut scene);
        assert_eq!(scene.shapes.len(), 1);
        assert_eq!((scene.shapes[0].ink, scene.shapes[0].alpha), (Ink::Muted, 0.5));
    });
}

