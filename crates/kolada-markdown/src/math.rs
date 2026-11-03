//! TeX math as notes write it between `$`s (see `Style::Math`) and `$$`s
//! (see `Block::Math`), read into a tree the view lays out (kolada-ui's
//! `math`). It reads what notes use, as Obsidian (MathJax) does: symbols,
//! scripts, fractions, roots, big operators, delimiters, accents, text,
//! spaces, matrices, cases and aligned lines. A command it does not know
//! is kept as an error, shown in place; a brace left open closes at the
//! end. Colours are read and left out: math is drawn in the note's ink.

#[cfg(test)]
mod tests;

/// How an atom spaces from its neighbours, as TeX's classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Ord,
    Op,
    Bin,
    Rel,
    Open,
    Close,
    Punct,
    Inner,
}

/// The face a symbol is set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Italic,
    Upright,
    Bold,
    BoldItalic,
    Mono,
}

/// A delimiter that stretches: brackets, bars, braces; or none (`\left.`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delim {
    None,
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Hat,
    Tilde,
    Bar,
    Vec,
    Dot,
    Ddot,
    Acute,
    Grave,
    Breve,
    Check,
    Overline,
    Underline,
    WideHat,
    WideTilde,
    OverBrace,
    UnderBrace,
    OverArrow,
    OverLeftArrow,
}

/// Math's four sizes: display (a block), text (in a line), and the
/// smaller script sizes scripts are set in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MathStyle {
    Display,
    Text,
    Script,
    ScriptScript,
}

/// How an array's columns sit and space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColAlign {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayKind {
    /// A matrix: columns a quad apart, set in text style.
    Matrix,
    /// Small, in script style (`smallmatrix`).
    Small,
    /// Cases: left-aligned columns, the second a quad from the first.
    Cases,
    /// Equations lined up at `&`: right and left columns in pairs, set in
    /// display style (`aligned`, `align`, `split`; `gathered` is centered).
    Aligned,
    /// An `array` with its own column spec.
    Array,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    /// A symbol or a run of letters set as one: what it reads, how it
    /// spaces, its face.
    Atom { text: String, class: Class, font: Font },
    /// Text (`\text{…}`), spaces kept.
    Text { text: String, font: Font },
    /// A big operator: its symbol (∑, ∫) or name (lim); whether its scripts
    /// go above and below it in display style (None: as the operator
    /// usually has them).
    BigOp { text: String, symbol: bool, limits: Option<bool> },
    /// Braces: what is inside, one atom.
    Group(Vec<Node>),
    Scripts { base: Box<Node>, sub: Option<Box<Node>>, sup: Option<Box<Node>> },
    /// A fraction; without a bar, a binomial (`delims` round it). `style`
    /// is the style it forces (`\dfrac`, `\tfrac`).
    Frac { num: Box<Node>, den: Box<Node>, bar: bool, delims: Option<(char, char)>, style: Option<MathStyle> },
    Sqrt { index: Option<Box<Node>>, body: Box<Node> },
    /// `\left( … \middle| … \right)`: delimiters as tall as what they hold.
    Fenced { left: Delim, right: Delim, body: Vec<Node> },
    Middle(Delim),
    /// `\big(` to `\Bigg(`: a delimiter of a size, 1 to 4.
    Big { delim: Delim, size: u8, class: Class },
    Accent { accent: Accent, body: Box<Node> },
    /// `\overset{over}{body}` (or under, `\underset`).
    Over { over: Box<Node>, body: Box<Node>, under: bool },
    /// Space, in ems (negative pulls together).
    Space(f32),
    Array { rows: Vec<Vec<Node>>, cols: Vec<ColAlign>, left: Delim, right: Delim, kind: ArrayKind, lines: Vec<usize> },
    /// `\displaystyle` and the like: the style of what follows in its list.
    Style(MathStyle),
    /// `\boxed{…}`: set on a soft panel.
    Boxed(Box<Node>),
    /// `\phantom{…}`: room, nothing drawn; whether it keeps the width and
    /// the height.
    Phantom { body: Box<Node>, width: bool, height: bool },
    /// What could not be read, as written.
    Error(String),
}

impl Node {
    /// An empty group (a missing argument).
    fn empty() -> Node {
        Node::Group(Vec::new())
    }
}

/// TeX math as one line of plain text (each part read once: linear in
/// the formula's size, however deep its scripts nest), for where it cannot be drawn (a
/// card, a list of notes): `\pi r^2` reads "πr²", `\frac{a}{b}` "a/b",
/// `\sqrt{x}` "√x".
pub fn linear(src: &str) -> String {
    let text = text_of(&parse(src));
    // Spaced as it is read, once.
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn text_of(nodes: &[Node]) -> String {
    nodes.iter().map(node_text).collect()
}

fn node_text(n: &Node) -> String {
    // What a script or a fraction's part reads, in brackets when it is more
    // than one thing.
    let part = |n: &Node| {
        let t = node_text(n);
        if t.chars().count() > 1 && t.chars().any(|c| !c.is_alphanumeric()) { format!("({t})") } else { t }
    };
    let char_of = |d: &Delim| match d {
        Delim::Char(c) => c.to_string(),
        Delim::None => String::new(),
    };
    match n {
        // A relation or an operator between two things, with room around it.
        Node::Atom { text, class: Class::Rel | Class::Bin, .. } => format!(" {text} "),
        Node::Atom { text, .. } | Node::Text { text, .. } | Node::BigOp { text, .. } | Node::Error(text) => text.clone(),
        Node::Fenced { left, right, body } => format!("{}{}{}", char_of(left), text_of(body), char_of(right)),
        Node::Group(v) => text_of(v),
        Node::Scripts { base, sub, sup } => {
            // Written out where it has no raised or lowered form: in
            // brackets when it is more than one letter ("x_(ab)").
            let written = |mark: char, t: String| if t.chars().count() > 1 { format!("{mark}({t})") } else { format!("{mark}{t}") };
            let mut out = node_text(base);
            if let Some(s) = sub {
                let t = node_text(s);
                out += &scripted(&t, false).unwrap_or_else(|| written('_', t));
            }
            if let Some(s) = sup {
                let t = node_text(s);
                out += &scripted(&t, true).unwrap_or_else(|| written('^', t));
            }
            // A sum or an integral stands apart from what it runs over.
            if matches!(**base, Node::BigOp { .. }) {
                out.push(' ');
            }
            out
        }
        Node::Frac { num, den, .. } => format!("{}/{}", part(num), part(den)),
        Node::Sqrt { body, .. } => format!("\u{221A}{}", part(body)),
        Node::Middle(d) | Node::Big { delim: d, .. } => char_of(d),
        Node::Accent { body, .. } | Node::Over { body, .. } | Node::Boxed(body) => node_text(body),
        Node::Space(em) => if *em > 0.2 { " ".to_string() } else { String::new() },
        Node::Array { rows, .. } => rows.iter().map(|r| r.iter().map(node_text).collect::<Vec<_>>().join(", ")).collect::<Vec<_>>().join("; "),
        Node::Style(_) | Node::Phantom { .. } => String::new(),
    }
}

/// `t` in superscript (or subscript) letters, where each has one.
fn scripted(t: &str, up: bool) -> Option<String> {
    // The letters Unicode has a raised or lowered form of; a script with
    // any other is written out (`^(…)`, `_(…)`).
    let (from, to) = if up {
        ("0123456789+-=()abdeghijklmnoprstuvwxyz", "\u{2070}\u{00B9}\u{00B2}\u{00B3}\u{2074}\u{2075}\u{2076}\u{2077}\u{2078}\u{2079}\u{207A}\u{207B}\u{207C}\u{207D}\u{207E}\u{1D43}\u{1D47}\u{1D48}\u{1D49}\u{1D4D}\u{02B0}\u{2071}\u{02B2}\u{1D4F}\u{02E1}\u{1D50}\u{207F}\u{1D52}\u{1D56}\u{02B3}\u{02E2}\u{1D57}\u{1D58}\u{1D5B}\u{02B7}\u{02E3}\u{02B8}\u{1DBB}")
    } else {
        ("0123456789+-=()aehijklmnoprstuvx", "\u{2080}\u{2081}\u{2082}\u{2083}\u{2084}\u{2085}\u{2086}\u{2087}\u{2088}\u{2089}\u{208A}\u{208B}\u{208C}\u{208D}\u{208E}\u{2090}\u{2091}\u{2095}\u{1D62}\u{2C7C}\u{2096}\u{2097}\u{2098}\u{2099}\u{2092}\u{209A}\u{1D63}\u{209B}\u{209C}\u{1D64}\u{1D65}\u{2093}")
    };
    // Spaces inside a script ("k = 1") go: the raised form has none.
    let t: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    let t = t.as_str();
    let to: Vec<char> = to.chars().collect();
    t.chars().map(|c| from.chars().position(|f| f == c).map(|i| to[i])).collect::<Option<String>>().filter(|s| !s.is_empty())
}

/// Reads TeX math into its list of nodes.
pub fn parse(src: &str) -> Vec<Node> {
    let mut p = Parser { s: src, at: 0, depth: 0 };
    let list = p.list(Stop::End);
    rows_to_array(list)
}

/// A top-level list with `\\` in it is lines, centred (or lined up at
/// `&`); with none, it is itself.
fn rows_to_array(list: Vec<Node>) -> Vec<Node> {
    if !list.iter().any(|n| matches!(n, Node::Error(e) if e == ROW || e == CELL)) {
        return list;
    }
    let aligned = list.iter().any(|n| matches!(n, Node::Error(e) if e == CELL));
    let mut rows = vec![Vec::new()];
    let mut cell: Vec<Node> = Vec::new();
    for n in list {
        match n {
            Node::Error(e) if e == ROW => {
                rows.last_mut().unwrap().push(Node::Group(std::mem::take(&mut cell)));
                rows.push(Vec::new());
            }
            Node::Error(e) if e == CELL => rows.last_mut().unwrap().push(Node::Group(std::mem::take(&mut cell))),
            n => cell.push(n),
        }
    }
    rows.last_mut().unwrap().push(Node::Group(cell));
    if rows.last().is_some_and(|r| r.len() == 1 && matches!(&r[0], Node::Group(g) if g.is_empty())) && rows.len() > 1 {
        rows.pop();
    }
    let (kind, cols) = if aligned { (ArrayKind::Aligned, Vec::new()) } else { (ArrayKind::Aligned, vec![ColAlign::Center]) };
    vec![Node::Array { rows, cols, left: Delim::None, right: Delim::None, kind, lines: Vec::new() }]
}

/// Markers a list hands back up to the array reading it: a `\\` and an `&`.
const ROW: &str = "\\\\";
const CELL: &str = "&";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stop {
    End,
    Brace,
    Bracket,
    Right,
    Env,
}

struct Parser<'a> {
    s: &'a str,
    at: usize,
    /// How deep the reading has gone: past a limit, the rest is an error
    /// (a note of ten thousand `{`s must not overflow the stack).
    depth: usize,
}

const MAX_DEPTH: usize = 64;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.s[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    /// Spaces and `%` comments.
    fn skip(&mut self) {
        loop {
            match self.peek() {
                Some(c) if c.is_whitespace() => {
                    self.bump();
                }
                Some('%') => {
                    let rest = &self.s[self.at..];
                    self.at += rest.find('\n').unwrap_or(rest.len());
                }
                _ => break,
            }
        }
    }

    /// A command's name after its backslash: letters, or one other
    /// character.
    fn name(&mut self) -> &'a str {
        let rest = &self.s[self.at..];
        let n = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let n = if n == 0 { rest.chars().next().map_or(0, char::len_utf8) } else { n };
        self.at += n;
        &rest[..n]
    }

    /// The command ahead, without reading it.
    fn peek_command(&self) -> Option<&'a str> {
        let rest = self.s[self.at..].strip_prefix('\\')?;
        let n = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let n = if n == 0 { rest.chars().next().map_or(0, char::len_utf8) } else { n };
        Some(&rest[..n])
    }

    fn list(&mut self, stop: Stop) -> Vec<Node> {
        self.depth += 1;
        let mut out: Vec<Node> = Vec::new();
        if self.depth > MAX_DEPTH {
            let rest = &self.s[self.at..];
            self.at = self.s.len();
            self.depth -= 1;
            return vec![Node::Error(rest.to_string())];
        }
        loop {
            self.skip();
            let Some(c) = self.peek() else { break };
            match c {
                '}' if stop == Stop::Brace => {
                    self.bump();
                    break;
                }
                // A stray closing brace: nothing.
                '}' => {
                    self.bump();
                }
                ']' if stop == Stop::Bracket => {
                    self.bump();
                    break;
                }
                '^' | '_' => {
                    self.bump();
                    let arg = self.arg();
                    attach(&mut out, c == '^', arg);
                }
                '\'' => {
                    let mut primes = String::new();
                    while self.peek() == Some('\'') {
                        self.bump();
                        primes.push('\u{2032}');
                    }
                    let primes = match primes.chars().count() {
                        2 => "\u{2033}".to_string(),
                        3 => "\u{2034}".to_string(),
                        _ => primes,
                    };
                    attach(&mut out, true, Node::Atom { text: primes, class: Class::Ord, font: Font::Upright });
                }
                '&' => {
                    self.bump();
                    out.push(Node::Error(CELL.into()));
                }
                '\\' => {
                    let save = self.at;
                    self.bump();
                    let name = self.name();
                    match name {
                        "right" if stop == Stop::Right => {
                            self.at = save;
                            break;
                        }
                        "end" if stop == Stop::Env => {
                            self.at = save;
                            break;
                        }
                        "\\" | "cr" | "newline" => {
                            // An optional gap after a row break (`\\[2pt]`).
                            self.skip();
                            if self.peek() == Some('[') && stop != Stop::Bracket {
                                let rest = &self.s[self.at..];
                                if let Some(k) = rest.find(']') {
                                    self.at += k + 1;
                                }
                            }
                            out.push(Node::Error(ROW.into()));
                        }
                        "over" | "choose" | "atop" => {
                            let num = Node::Group(std::mem::take(&mut out));
                            let den = Node::Group(self.list_until(stop));
                            let (bar, delims) = match name {
                                "over" => (true, None),
                                "choose" => (false, Some(('(', ')'))),
                                _ => (false, None),
                            };
                            out.push(Node::Frac { num: Box::new(num), den: Box::new(den), bar, delims, style: None });
                        }
                        "limits" | "nolimits" | "displaylimits" => {
                            if let Some(Node::BigOp { limits, .. }) = last_op(&mut out) {
                                *limits = match name {
                                    "limits" => Some(true),
                                    "nolimits" => Some(false),
                                    _ => None,
                                };
                            }
                        }
                        _ => {
                            let node = self.command(name);
                            if let Some(node) = node {
                                out.push(node);
                            }
                        }
                    }
                }
                _ => {
                    let node = self.atom();
                    if let Some(node) = node {
                        out.push(node);
                    }
                }
            }
        }
        self.depth -= 1;
        out
    }

    /// The rest of a list, from an `\over` in it, up to where the list
    /// stops (the stop itself left to the list).
    fn list_until(&mut self, stop: Stop) -> Vec<Node> {
        let start = self.at;
        let list = self.list(match stop {
            Stop::Brace => Stop::Brace,
            other => other,
        });
        // The brace that ended it belongs to the list around.
        if stop == Stop::Brace && self.at > start && self.s[..self.at].ends_with('}') {
            self.at -= 1;
        }
        list
    }

    /// One character that is not a command.
    fn atom(&mut self) -> Option<Node> {
        let c = self.bump()?;
        Some(match c {
            '{' => Node::Group(self.list(Stop::Brace)),
            '~' => Node::Space(SPACE),
            'a'..='z' | 'A'..='Z' => Node::Atom { text: c.to_string(), class: Class::Ord, font: Font::Italic },
            '0'..='9' | '.' => {
                // A number reads as one: 3.14 sets with no gaps.
                let mut text = c.to_string();
                while let Some(d) = self.peek().filter(|d| d.is_ascii_digit() || (*d == '.' && text.chars().last().is_some_and(|l| l.is_ascii_digit()))) {
                    self.bump();
                    text.push(d);
                }
                Node::Atom { text, class: Class::Ord, font: Font::Upright }
            }
            '+' | '*' => Node::Atom { text: if c == '*' { "\u{2217}".into() } else { "+".into() }, class: Class::Bin, font: Font::Upright },
            '-' => Node::Atom { text: "\u{2212}".into(), class: Class::Bin, font: Font::Upright },
            '=' | '<' | '>' | ':' => Node::Atom { text: c.to_string(), class: Class::Rel, font: Font::Upright },
            ',' | ';' => Node::Atom { text: c.to_string(), class: Class::Punct, font: Font::Upright },
            '(' | '[' => Node::Atom { text: c.to_string(), class: Class::Open, font: Font::Upright },
            ')' | ']' => Node::Atom { text: c.to_string(), class: Class::Close, font: Font::Upright },
            '!' | '?' => Node::Atom { text: c.to_string(), class: Class::Close, font: Font::Upright },
            '|' => Node::Atom { text: "|".into(), class: Class::Ord, font: Font::Upright },
            '/' => Node::Atom { text: "/".into(), class: Class::Ord, font: Font::Upright },
            '#' | '$' => Node::Atom { text: c.to_string(), class: Class::Ord, font: Font::Upright },
            // Big operators typed as themselves.
            '∑' | '∏' | '∐' | '∫' | '∬' | '∭' | '∮' | '⋃' | '⋂' | '⋁' | '⋀' | '⨁' | '⨂' => Node::BigOp { text: c.to_string(), symbol: true, limits: None },
            c => match symbol_class(c) {
                Some(class) => Node::Atom { text: c.to_string(), class, font: Font::Upright },
                // Letters of any script, written as they are.
                None if c.is_alphabetic() => Node::Atom { text: c.to_string(), class: Class::Ord, font: Font::Italic },
                None => Node::Atom { text: c.to_string(), class: Class::Ord, font: Font::Upright },
            },
        })
    }

    /// A command's argument: a group in braces, or one token.
    fn arg(&mut self) -> Node {
        self.skip();
        self.depth += 1;
        let node = match self.peek() {
            _ if self.depth > MAX_DEPTH => {
                let rest = &self.s[self.at..];
                self.at = self.s.len();
                Node::Error(rest.to_string())
            }
            None | Some('}') => Node::empty(),
            Some('{') => {
                self.bump();
                Node::Group(self.list(Stop::Brace))
            }
            Some('\\') => {
                self.bump();
                let name = self.name();
                self.command(name).unwrap_or_else(Node::empty)
            }
            // A single digit, even of a number: `x^23` is x² then 3.
            Some(c) if c.is_ascii_digit() => {
                self.bump();
                Node::Atom { text: c.to_string(), class: Class::Ord, font: Font::Upright }
            }
            Some(_) => self.atom().unwrap_or_else(Node::empty),
        };
        self.depth -= 1;
        node
    }

    /// An optional argument in brackets, if one follows.
    fn optional(&mut self) -> Option<Node> {
        self.skip();
        if self.peek() != Some('[') {
            return None;
        }
        self.bump();
        Some(Node::Group(self.list(Stop::Bracket)))
    }

    /// A group's text, as written: `\text{…}`, `\begin{…}`, a colour.
    fn raw(&mut self) -> String {
        self.skip();
        if self.peek() != Some('{') {
            return self.bump().map(String::from).unwrap_or_default();
        }
        self.bump();
        let rest = &self.s[self.at..];
        let mut level = 0;
        let mut end = rest.len();
        let mut chars = rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '{' => level += 1,
                '}' if level == 0 => {
                    end = i;
                    break;
                }
                '}' => level -= 1,
                _ => {}
            }
        }
        let text = rest[..end].to_string();
        self.at += (end + 1).min(rest.len());
        text
    }

    /// A delimiter after `\left`, `\big` and the like.
    fn delim(&mut self) -> Delim {
        self.skip();
        match self.peek() {
            Some('\\') => {
                self.bump();
                let name = self.name();
                match name {
                    "{" | "lbrace" => Delim::Char('{'),
                    "}" | "rbrace" => Delim::Char('}'),
                    "|" | "Vert" | "lVert" | "rVert" => Delim::Char('\u{2016}'),
                    "vert" | "lvert" | "rvert" | "mid" => Delim::Char('|'),
                    "langle" => Delim::Char('\u{27E8}'),
                    "rangle" => Delim::Char('\u{27E9}'),
                    "lfloor" => Delim::Char('\u{230A}'),
                    "rfloor" => Delim::Char('\u{230B}'),
                    "lceil" => Delim::Char('\u{2308}'),
                    "rceil" => Delim::Char('\u{2309}'),
                    "lbrack" => Delim::Char('['),
                    "rbrack" => Delim::Char(']'),
                    "backslash" => Delim::Char('\\'),
                    "uparrow" => Delim::Char('\u{2191}'),
                    "downarrow" => Delim::Char('\u{2193}'),
                    _ => Delim::None,
                }
            }
            Some('.') => {
                self.bump();
                Delim::None
            }
            Some(c @ ('(' | ')' | '[' | ']' | '|' | '/' | '<' | '>')) => {
                self.bump();
                Delim::Char(match c {
                    '<' => '\u{27E8}',
                    '>' => '\u{27E9}',
                    c => c,
                })
            }
            Some(c @ ('\u{27E8}' | '\u{27E9}' | '\u{2016}' | '\u{230A}' | '\u{230B}' | '\u{2308}' | '\u{2309}')) => {
                self.bump();
                Delim::Char(c)
            }
            _ => Delim::None,
        }
    }

    /// The node a command makes; None for one that only reads its
    /// arguments (`\label`, colours).
    fn command(&mut self, name: &str) -> Option<Node> {
        if let Some((c, class)) = symbol(name) {
            let font = if is_lower_greek(c) { Font::Italic } else { Font::Upright };
            return Some(Node::Atom { text: c.to_string(), class, font });
        }
        if let Some((text, symbol, limits)) = big_op(name) {
            return Some(Node::BigOp { text: text.to_string(), symbol, limits });
        }
        if FUNCTIONS.contains(&name) {
            return Some(Node::Atom { text: name.to_string(), class: Class::Op, font: Font::Upright });
        }
        if let Some(em) = space(name) {
            return Some(Node::Space(em));
        }
        if let Some(accent) = accent(name) {
            return Some(Node::Accent { accent, body: Box::new(self.arg()) });
        }
        let node = match name {
            "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" => {
                let num = Box::new(self.arg());
                let den = Box::new(self.arg());
                let style = match name {
                    "dfrac" | "cfrac" | "dbinom" => Some(MathStyle::Display),
                    "tfrac" | "tbinom" => Some(MathStyle::Text),
                    _ => None,
                };
                let binom = name.ends_with("binom");
                Node::Frac { num, den, bar: !binom, delims: binom.then_some(('(', ')')), style }
            }
            "sqrt" => {
                let index = self.optional().map(Box::new);
                Node::Sqrt { index, body: Box::new(self.arg()) }
            }
            "left" => {
                let left = self.delim();
                let body = self.list(Stop::Right);
                // The `\right` the list stopped at, and its delimiter.
                let right = if self.peek_command() == Some("right") {
                    self.bump();
                    self.name();
                    self.delim()
                } else {
                    Delim::None
                };
                Node::Fenced { left, right, body }
            }
            "middle" => Node::Middle(self.delim()),
            "right" => {
                // A `\right` with no `\left`: its delimiter as it is.
                return match self.delim() {
                    Delim::Char(c) => Some(Node::Atom { text: c.to_string(), class: Class::Close, font: Font::Upright }),
                    Delim::None => None,
                };
            }
            "big" | "Big" | "bigg" | "Bigg" | "bigl" | "Bigl" | "biggl" | "Biggl" | "bigr" | "Bigr" | "biggr" | "Biggr" | "bigm" | "Bigm" | "biggm" | "Biggm" => {
                let size = match name.trim_end_matches(['l', 'r', 'm']) {
                    "big" => 1,
                    "Big" => 2,
                    "bigg" => 3,
                    _ => 4,
                };
                let class = match name.as_bytes().last() {
                    Some(b'l') => Class::Open,
                    Some(b'r') => Class::Close,
                    Some(b'm') => Class::Rel,
                    _ => Class::Ord,
                };
                Node::Big { delim: self.delim(), size, class }
            }
            "text" | "textrm" | "textnormal" | "mbox" | "hbox" | "textup" | "textsf" => Node::Text { text: unescape_text(&self.raw()), font: Font::Upright },
            "textbf" => Node::Text { text: unescape_text(&self.raw()), font: Font::Bold },
            "textit" | "emph" => Node::Text { text: unescape_text(&self.raw()), font: Font::Italic },
            "texttt" => Node::Text { text: unescape_text(&self.raw()), font: Font::Mono },
            "mathrm" | "mathup" | "rm" | "mathsf" | "mathnormal" | "mathit" | "mathbf" | "boldsymbol" | "bm" | "mathtt" | "bf" | "it" => {
                let font = match name {
                    "mathbf" | "bf" => Font::Bold,
                    "boldsymbol" | "bm" => Font::BoldItalic,
                    "mathit" | "it" | "mathnormal" => Font::Italic,
                    "mathtt" => Font::Mono,
                    _ => Font::Upright,
                };
                let mut body = self.arg();
                set_font(&mut body, font);
                body
            }
            "mathbb" | "Bbb" | "mathcal" | "mathscr" | "mathfrak" => {
                let mut body = self.arg();
                letters(&mut body, name);
                body
            }
            "operatorname" | "operatornamewithlimits" => {
                // `\operatorname*{…}` takes limits.
                self.skip();
                let star = self.peek() == Some('*');
                if star {
                    self.bump();
                }
                let text = unescape_text(&self.raw()).replace(' ', "");
                if star || name == "operatornamewithlimits" { Node::BigOp { text, symbol: false, limits: None } } else { Node::Atom { text, class: Class::Op, font: Font::Upright } }
            }
            "overset" | "underset" | "stackrel" => {
                let over = Box::new(self.arg());
                let body = Box::new(self.arg());
                let mut body = body;
                if name == "stackrel" {
                    body = Box::new(Node::Group(vec![*body]));
                }
                Node::Over { over, body, under: name == "underset" }
            }
            "boxed" | "fbox" | "colorbox" => {
                if name == "colorbox" {
                    self.raw();
                }
                Node::Boxed(Box::new(self.arg()))
            }
            "phantom" | "hphantom" | "vphantom" => Node::Phantom { body: Box::new(self.arg()), width: name != "vphantom", height: name != "hphantom" },
            "color" => {
                self.raw();
                return None;
            }
            "textcolor" => {
                self.raw();
                return Some(self.arg());
            }
            "label" | "nonumber" | "notag" => {
                if name == "label" {
                    self.raw();
                }
                return None;
            }
            "tag" => {
                let star = self.peek() == Some('*');
                if star {
                    self.bump();
                }
                let tag = unescape_text(&self.raw());
                return Some(Node::Group(vec![Node::Space(2.0), Node::Text { text: if star { tag } else { format!("({tag})") }, font: Font::Upright }]));
            }
            "displaystyle" => Node::Style(MathStyle::Display),
            "textstyle" => Node::Style(MathStyle::Text),
            "scriptstyle" => Node::Style(MathStyle::Script),
            "scriptscriptstyle" => Node::Style(MathStyle::ScriptScript),
            "hspace" | "hskip" | "kern" | "mkern" | "mskip" | "mspace" => {
                self.skip();
                if self.peek() == Some('*') {
                    self.bump();
                }
                let len = if self.peek() == Some('{') { self.raw() } else { self.length_text() };
                Node::Space(length(&len))
            }
            "not" => {
                self.skip();
                let next = self.arg();
                negate(next)
            }
            "pmod" => {
                let arg = self.arg();
                Node::Group(vec![
                    Node::Space(1.0),
                    Node::Atom { text: "(".into(), class: Class::Open, font: Font::Upright },
                    Node::Atom { text: "mod".into(), class: Class::Ord, font: Font::Upright },
                    Node::Space(6.0 / 18.0),
                    arg,
                    Node::Atom { text: ")".into(), class: Class::Close, font: Font::Upright },
                ])
            }
            "bmod" => Node::Atom { text: "mod".into(), class: Class::Bin, font: Font::Upright },
            "mod" => Node::Group(vec![Node::Space(1.0), Node::Atom { text: "mod".into(), class: Class::Ord, font: Font::Upright }, Node::Space(6.0 / 18.0)]),
            "begin" => {
                let env = self.raw();
                self.environment(env.trim())
            }
            "end" => {
                // An `\end` nothing began.
                self.raw();
                return None;
            }
            "{" | "lbrace" => Node::Atom { text: "{".into(), class: Class::Open, font: Font::Upright },
            "}" | "rbrace" => Node::Atom { text: "}".into(), class: Class::Close, font: Font::Upright },
            "$" | "%" | "&" | "#" | "_" => Node::Atom { text: name.into(), class: Class::Ord, font: Font::Upright },
            "|" => Node::Atom { text: "\u{2016}".into(), class: Class::Ord, font: Font::Upright },
            "" => return None,
            _ => Node::Error(format!("\\{name}")),
        };
        Some(node)
    }

    /// A length written without braces after `\kern`: `3mu`, `-0.5em`.
    fn length_text(&mut self) -> String {
        self.skip();
        let rest = &self.s[self.at..];
        let n = rest.bytes().take_while(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+')).count();
        let unit = rest[n..].bytes().take_while(u8::is_ascii_alphabetic).count().min(2);
        self.at += n + unit;
        rest[..n + unit].to_string()
    }

    fn environment(&mut self, env: &str) -> Node {
        let name = env.trim_end_matches('*');
        let (kind, left, right) = match name {
            "matrix" => (ArrayKind::Matrix, Delim::None, Delim::None),
            "pmatrix" => (ArrayKind::Matrix, Delim::Char('('), Delim::Char(')')),
            "bmatrix" => (ArrayKind::Matrix, Delim::Char('['), Delim::Char(']')),
            "Bmatrix" => (ArrayKind::Matrix, Delim::Char('{'), Delim::Char('}')),
            "vmatrix" => (ArrayKind::Matrix, Delim::Char('|'), Delim::Char('|')),
            "Vmatrix" => (ArrayKind::Matrix, Delim::Char('\u{2016}'), Delim::Char('\u{2016}')),
            "smallmatrix" => (ArrayKind::Small, Delim::None, Delim::None),
            "cases" | "dcases" => (ArrayKind::Cases, Delim::Char('{'), Delim::None),
            "rcases" => (ArrayKind::Cases, Delim::None, Delim::Char('}')),
            "aligned" | "align" | "split" | "alignat" | "alignedat" | "eqnarray" | "flalign" => (ArrayKind::Aligned, Delim::None, Delim::None),
            "gathered" | "gather" | "equation" | "multline" | "displaymath" => (ArrayKind::Aligned, Delim::None, Delim::None),
            "array" | "darray" | "subarray" => (ArrayKind::Array, Delim::None, Delim::None),
            _ => (ArrayKind::Matrix, Delim::None, Delim::None),
        };
        let mut cols = Vec::new();
        if kind == ArrayKind::Array {
            for c in self.raw().chars() {
                match c {
                    'l' => cols.push(ColAlign::Left),
                    'c' => cols.push(ColAlign::Center),
                    'r' => cols.push(ColAlign::Right),
                    _ => {}
                }
            }
        }
        if matches!(name, "alignat" | "alignedat") {
            self.raw();
        }
        if matches!(name, "gathered" | "gather" | "equation" | "multline" | "displaymath") {
            cols = vec![ColAlign::Center];
        }
        let list = self.list(Stop::Env);
        // The `\end{…}` the list stopped at.
        if self.peek_command() == Some("end") {
            self.bump();
            self.name();
            self.raw();
        }
        let mut rows: Vec<Vec<Node>> = vec![Vec::new()];
        let mut cell = Vec::new();
        let mut lines = Vec::new();
        for n in list {
            match n {
                Node::Error(e) if e == ROW => {
                    rows.last_mut().unwrap().push(Node::Group(std::mem::take(&mut cell)));
                    rows.push(Vec::new());
                }
                Node::Error(e) if e == CELL => rows.last_mut().unwrap().push(Node::Group(std::mem::take(&mut cell))),
                Node::Error(e) if e == "\\hline" => lines.push(rows.len() - 1),
                n => cell.push(n),
            }
        }
        rows.last_mut().unwrap().push(Node::Group(cell));
        // A last `\\` ends a row; it starts none.
        if rows.len() > 1 && rows.last().is_some_and(|r| r.len() == 1 && matches!(&r[0], Node::Group(g) if g.is_empty())) {
            rows.pop();
        }
        Node::Array { rows, cols, left, right, kind, lines }
    }
}

/// Puts `arg` on the last node as its superscript or subscript.
fn attach(out: &mut Vec<Node>, sup: bool, arg: Node) {
    let base = match out.pop() {
        Some(Node::Scripts { base, sub, sup: s }) => {
            // A prime then `^`: one superscript, the prime first (x'^2).
            match (sup, s, sub) {
                (true, Some(s), sub) => {
                    let mut joined = match *s {
                        Node::Group(g) => g,
                        n => vec![n],
                    };
                    joined.push(arg);
                    out.push(Node::Scripts { base, sub, sup: Some(Box::new(Node::Group(joined))) });
                    return;
                }
                (true, None, sub) => {
                    out.push(Node::Scripts { base, sub, sup: Some(Box::new(arg)) });
                    return;
                }
                (false, s, None) => {
                    out.push(Node::Scripts { base, sub: Some(Box::new(arg)), sup: s });
                    return;
                }
                // A second subscript: on what has the first.
                (false, s, Some(sub)) => Node::Scripts { base, sub: Some(sub), sup: s },
            }
        }
        Some(Node::Style(s)) => {
            out.push(Node::Style(s));
            Node::empty()
        }
        Some(n) => n,
        None => Node::empty(),
    };
    let (sub, sup) = if sup { (None, Some(Box::new(arg))) } else { (Some(Box::new(arg)), None) };
    out.push(Node::Scripts { base: Box::new(base), sub, sup });
}

/// The big operator a list ends with, for `\limits` after it.
fn last_op(out: &mut [Node]) -> Option<&mut Node> {
    match out.last_mut()? {
        n @ Node::BigOp { .. } => Some(n),
        Node::Scripts { base, .. } => match base.as_mut() {
            n @ Node::BigOp { .. } => Some(n),
            _ => None,
        },
        _ => None,
    }
}

/// Sets every letter of `node` in `font`.
fn set_font(node: &mut Node, font: Font) {
    match node {
        Node::Atom { font: f, class, text } => {
            // Bold leaves symbols as they are, and only digits and letters
            // turn upright.
            let letters = text.chars().all(char::is_alphanumeric);
            if letters || *class == Class::Ord {
                *f = match (font, *f) {
                    (Font::Bold, Font::Italic) if is_lower_greek(text.chars().next().unwrap_or(' ')) => Font::BoldItalic,
                    _ => font,
                };
            }
        }
        Node::Group(list) => list.iter_mut().for_each(|n| set_font(n, font)),
        Node::Scripts { base, .. } => set_font(base, font),
        Node::Text { font: f, .. } => *f = font,
        _ => {}
    }
}

/// Sets every letter of `node` in a style Unicode has letters for:
/// double-struck (ℝ), script (𝒜), fraktur (𝔄).
fn letters(node: &mut Node, command: &str) {
    match node {
        Node::Atom { text, font, .. } => {
            *text = text.chars().map(|c| styled_letter(c, command)).collect();
            *font = Font::Upright;
        }
        Node::Group(list) => list.iter_mut().for_each(|n| letters(n, command)),
        Node::Scripts { base, .. } => letters(base, command),
        _ => {}
    }
}

fn styled_letter(c: char, command: &str) -> char {
    let (upper, lower, holes): (u32, u32, &[(char, char)]) = match command {
        "mathbb" | "Bbb" => (0x1D538, 0x1D552, &[('C', 'ℂ'), ('H', 'ℍ'), ('N', 'ℕ'), ('P', 'ℙ'), ('Q', 'ℚ'), ('R', 'ℝ'), ('Z', 'ℤ')]),
        "mathcal" | "mathscr" => (0x1D49C, 0x1D4B6, &[('B', 'ℬ'), ('E', 'ℰ'), ('F', 'ℱ'), ('H', 'ℋ'), ('I', 'ℐ'), ('L', 'ℒ'), ('M', 'ℳ'), ('R', 'ℛ'), ('e', 'ℯ'), ('g', 'ℊ'), ('o', 'ℴ')]),
        _ => (0x1D504, 0x1D51E, &[('C', 'ℭ'), ('H', 'ℌ'), ('I', 'ℑ'), ('R', 'ℜ'), ('Z', 'ℨ')]),
    };
    if let Some(&(_, h)) = holes.iter().find(|(k, _)| *k == c) {
        return h;
    }
    let code = match c {
        'A'..='Z' => upper + (c as u32 - 'A' as u32),
        'a'..='z' => lower + (c as u32 - 'a' as u32),
        '0'..='9' if command == "mathbb" || command == "Bbb" => 0x1D7D8 + (c as u32 - '0' as u32),
        _ => return c,
    };
    char::from_u32(code).unwrap_or(c)
}

/// `\not` before a relation: its negated form.
fn negate(node: Node) -> Node {
    let Node::Atom { text, class, font } = node else { return node };
    let negated = match text.as_str() {
        "=" => "\u{2260}",
        "<" => "\u{226E}",
        ">" => "\u{226F}",
        "\u{2208}" => "\u{2209}",
        "\u{220B}" => "\u{220C}",
        "\u{2264}" => "\u{2270}",
        "\u{2265}" => "\u{2271}",
        "\u{2261}" => "\u{2262}",
        "\u{223C}" => "\u{2241}",
        "\u{2248}" => "\u{2249}",
        "\u{2282}" => "\u{2284}",
        "\u{2283}" => "\u{2285}",
        "\u{2286}" => "\u{2288}",
        "\u{2287}" => "\u{2289}",
        "\u{2203}" => "\u{2204}",
        "\u{2225}" => "\u{2226}",
        "\u{2223}" => "\u{2224}",
        _ => return Node::Atom { text: format!("{text}\u{0338}"), class, font },
    };
    Node::Atom { text: negated.into(), class, font }
}

/// A space's width in ems: `\quad` and friends.
fn space(name: &str) -> Option<f32> {
    Some(match name {
        "," | "thinspace" => 3.0 / 18.0,
        ":" | ">" | "medspace" => 4.0 / 18.0,
        ";" | "thickspace" => 5.0 / 18.0,
        "!" | "negthinspace" => -3.0 / 18.0,
        "negmedspace" => -4.0 / 18.0,
        "negthickspace" => -5.0 / 18.0,
        " " | "space" | "nobreakspace" => SPACE,
        "enspace" | "enskip" => 0.5,
        "quad" => 1.0,
        "qquad" => 2.0,
        _ => return None,
    })
}

/// A space typed as `~` or `\ `.
const SPACE: f32 = 0.25;

/// A length as `\hspace` takes it, in ems: em, ex, mu, pt and the like.
fn length(s: &str) -> f32 {
    let s = s.trim();
    let n = s.bytes().take_while(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+')).count();
    let value: f32 = s[..n].parse().unwrap_or(0.0);
    let em = match s[n..].trim() {
        "ex" => 0.43,
        "mu" => 1.0 / 18.0,
        "pt" => 0.1,
        "px" => 1.0 / 16.0,
        "mm" => 0.284,
        "cm" => 2.84,
        "in" => 7.2,
        _ => 1.0,
    };
    (value * em).clamp(-20.0, 20.0)
}

/// Text in `\text{…}` as it reads: `\$`, `\%`, `\{` and `\ ` as themselves.
fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(n) if !n.is_ascii_alphabetic() => out.push(n),
                Some(n) => {
                    // A command in text: its letters are its name, left out.
                    let mut rest = chars.clone();
                    let mut skipped = n.to_string();
                    while let Some(m) = rest.clone().next().filter(char::is_ascii_alphabetic) {
                        rest.next();
                        skipped.push(m);
                    }
                    chars = rest;
                    if let Some((c, _)) = symbol(&skipped) {
                        out.push(c);
                    }
                }
                None => {}
            },
            '{' | '}' => {}
            '~' => out.push('\u{00A0}'),
            c => out.push(c),
        }
    }
    out
}

fn is_lower_greek(c: char) -> bool {
    matches!(c, '\u{03B1}'..='\u{03C9}' | '\u{03D1}' | '\u{03D5}' | '\u{03D6}' | '\u{03F1}' | '\u{03F5}')
}

/// The class of a symbol typed as itself (≤, →, ∑…).
fn symbol_class(c: char) -> Option<Class> {
    SYMBOLS.iter().find(|(_, s, _)| *s == c).map(|(_, _, class)| *class)
}

/// A symbol command: its character and class.
fn symbol(name: &str) -> Option<(char, Class)> {
    SYMBOLS.iter().find(|(n, _, _)| *n == name).map(|(_, c, class)| (*c, *class))
}

/// Big operators: the symbol or name, whether a symbol, its limits.
fn big_op(name: &str) -> Option<(&'static str, bool, Option<bool>)> {
    let symbol = |s| Some((s, true, None));
    match name {
        "sum" => symbol("\u{2211}"),
        "prod" => symbol("\u{220F}"),
        "coprod" => symbol("\u{2210}"),
        "int" => symbol("\u{222B}"),
        "iint" => symbol("\u{222C}"),
        "iiint" => symbol("\u{222D}"),
        "oint" => symbol("\u{222E}"),
        "bigcup" => symbol("\u{22C3}"),
        "bigcap" => symbol("\u{22C2}"),
        "bigvee" => symbol("\u{22C1}"),
        "bigwedge" => symbol("\u{22C0}"),
        "bigoplus" => symbol("\u{2A01}"),
        "bigotimes" => symbol("\u{2A02}"),
        "bigodot" => symbol("\u{2A00}"),
        "bigsqcup" => symbol("\u{2A06}"),
        "biguplus" => symbol("\u{2A04}"),
        "lim" | "liminf" | "limsup" | "max" | "min" | "sup" | "inf" | "det" | "gcd" | "Pr" | "argmax" | "argmin" => {
            let text = match name {
                "liminf" => "lim inf",
                "limsup" => "lim sup",
                "argmax" => "arg max",
                "argmin" => "arg min",
                n => FUNCTIONS_WITH_LIMITS.iter().find(|f| **f == n).copied().unwrap_or("lim"),
            };
            Some((text, false, None))
        }
        _ => None,
    }
}

const FUNCTIONS_WITH_LIMITS: &[&str] = &["lim", "max", "min", "sup", "inf", "det", "gcd", "Pr"];

/// Functions set upright, spaced as operators, scripts beside them.
const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "cot", "sec", "csc", "arcsin", "arccos", "arctan", "sinh", "cosh", "tanh", "coth", "log", "ln", "lg", "exp", "dim", "ker", "deg", "hom",
    "arg", "sgn", "tr", "rank", "Tr", "span", "erf",
];

fn accent(name: &str) -> Option<Accent> {
    Some(match name {
        "hat" => Accent::Hat,
        "widehat" => Accent::WideHat,
        "tilde" => Accent::Tilde,
        "widetilde" => Accent::WideTilde,
        "bar" => Accent::Bar,
        "vec" => Accent::Vec,
        "dot" => Accent::Dot,
        "ddot" => Accent::Ddot,
        "acute" => Accent::Acute,
        "grave" => Accent::Grave,
        "breve" => Accent::Breve,
        "check" => Accent::Check,
        "overline" => Accent::Overline,
        "underline" => Accent::Underline,
        "overbrace" => Accent::OverBrace,
        "underbrace" => Accent::UnderBrace,
        "overrightarrow" => Accent::OverArrow,
        "overleftarrow" => Accent::OverLeftArrow,
        _ => return None,
    })
}

/// Symbol commands: name, character, class.
const SYMBOLS: &[(&str, char, Class)] = {
    use Class::*;
    &[
        // Greek: lowercase italic (set in `command`), uppercase upright.
        ("alpha", 'α', Ord), ("beta", 'β', Ord), ("gamma", 'γ', Ord), ("delta", 'δ', Ord), ("epsilon", 'ϵ', Ord), ("varepsilon", 'ε', Ord),
        ("zeta", 'ζ', Ord), ("eta", 'η', Ord), ("theta", 'θ', Ord), ("vartheta", 'ϑ', Ord), ("iota", 'ι', Ord), ("kappa", 'κ', Ord),
        ("lambda", 'λ', Ord), ("mu", 'μ', Ord), ("nu", 'ν', Ord), ("xi", 'ξ', Ord), ("omicron", 'ο', Ord), ("pi", 'π', Ord), ("varpi", 'ϖ', Ord),
        ("rho", 'ρ', Ord), ("varrho", 'ϱ', Ord), ("sigma", 'σ', Ord), ("varsigma", 'ς', Ord), ("tau", 'τ', Ord), ("upsilon", 'υ', Ord),
        ("phi", 'ϕ', Ord), ("varphi", 'φ', Ord), ("chi", 'χ', Ord), ("psi", 'ψ', Ord), ("omega", 'ω', Ord),
        ("Gamma", 'Γ', Ord), ("Delta", 'Δ', Ord), ("Theta", 'Θ', Ord), ("Lambda", 'Λ', Ord), ("Xi", 'Ξ', Ord), ("Pi", 'Π', Ord),
        ("Sigma", 'Σ', Ord), ("Upsilon", 'Υ', Ord), ("Phi", 'Φ', Ord), ("Psi", 'Ψ', Ord), ("Omega", 'Ω', Ord),
        // Binary operators.
        ("pm", '±', Bin), ("mp", '∓', Bin), ("times", '×', Bin), ("div", '÷', Bin), ("cdot", '⋅', Bin), ("ast", '∗', Bin), ("star", '⋆', Bin),
        ("circ", '∘', Bin), ("bullet", '∙', Bin), ("cup", '∪', Bin), ("cap", '∩', Bin), ("wedge", '∧', Bin), ("land", '∧', Bin),
        ("vee", '∨', Bin), ("lor", '∨', Bin), ("setminus", '∖', Bin), ("oplus", '⊕', Bin), ("ominus", '⊖', Bin), ("otimes", '⊗', Bin),
        ("oslash", '⊘', Bin), ("odot", '⊙', Bin), ("sqcup", '⊔', Bin), ("sqcap", '⊓', Bin), ("uplus", '⊎', Bin), ("amalg", '⨿', Bin),
        ("dagger", '†', Bin), ("ddagger", '‡', Bin), ("wr", '≀', Bin), ("diamond", '⋄', Bin), ("triangleleft", '◁', Bin), ("triangleright", '▷', Bin),
        // Relations.
        ("leq", '≤', Rel), ("le", '≤', Rel), ("geq", '≥', Rel), ("ge", '≥', Rel), ("neq", '≠', Rel), ("ne", '≠', Rel), ("leqslant", '⩽', Rel),
        ("geqslant", '⩾', Rel), ("approx", '≈', Rel), ("equiv", '≡', Rel), ("sim", '∼', Rel), ("simeq", '≃', Rel), ("cong", '≅', Rel),
        ("propto", '∝', Rel), ("in", '∈', Rel), ("notin", '∉', Rel), ("ni", '∋', Rel), ("owns", '∋', Rel), ("subset", '⊂', Rel),
        ("subseteq", '⊆', Rel), ("supset", '⊃', Rel), ("supseteq", '⊇', Rel), ("subsetneq", '⊊', Rel), ("supsetneq", '⊋', Rel),
        ("sqsubseteq", '⊑', Rel), ("sqsupseteq", '⊒', Rel), ("to", '→', Rel), ("rightarrow", '→', Rel), ("leftarrow", '←', Rel), ("gets", '←', Rel),
        ("leftrightarrow", '↔', Rel), ("Rightarrow", '⇒', Rel), ("Leftarrow", '⇐', Rel), ("Leftrightarrow", '⇔', Rel), ("implies", '⟹', Rel),
        ("impliedby", '⟸', Rel), ("iff", '⟺', Rel), ("longrightarrow", '⟶', Rel), ("longleftarrow", '⟵', Rel), ("Longrightarrow", '⟹', Rel),
        ("Longleftarrow", '⟸', Rel), ("longleftrightarrow", '⟷', Rel), ("Longleftrightarrow", '⟺', Rel), ("mapsto", '↦', Rel),
        ("longmapsto", '⟼', Rel), ("hookrightarrow", '↪', Rel), ("hookleftarrow", '↩', Rel), ("uparrow", '↑', Rel), ("downarrow", '↓', Rel),
        ("updownarrow", '↕', Rel), ("Uparrow", '⇑', Rel), ("Downarrow", '⇓', Rel), ("nearrow", '↗', Rel), ("searrow", '↘', Rel),
        ("nwarrow", '↖', Rel), ("swarrow", '↙', Rel), ("rightleftharpoons", '⇌', Rel), ("leftrightharpoons", '⇋', Rel),
        ("rightharpoonup", '⇀', Rel), ("leftharpoonup", '↼', Rel), ("perp", '⊥', Rel), ("parallel", '∥', Rel), ("mid", '∣', Rel),
        ("nmid", '∤', Rel), ("ll", '≪', Rel), ("gg", '≫', Rel), ("prec", '≺', Rel), ("succ", '≻', Rel), ("preceq", '⪯', Rel), ("succeq", '⪰', Rel),
        ("vdash", '⊢', Rel), ("dashv", '⊣', Rel), ("models", '⊨', Rel), ("asymp", '≍', Rel), ("doteq", '≐', Rel), ("triangleq", '≜', Rel),
        ("coloneqq", '≔', Rel), ("defeq", '≝', Rel), ("lesssim", '≲', Rel), ("gtrsim", '≳', Rel), ("smile", '⌣', Rel), ("frown", '⌢', Rel),
        ("bowtie", '⋈', Rel), ("vDash", '⊨', Rel), ("nleq", '≰', Rel), ("ngeq", '≱', Rel), ("nsubseteq", '⊈', Rel),
        // Ordinary symbols.
        ("infty", '∞', Ord), ("partial", '∂', Ord), ("nabla", '∇', Ord), ("forall", '∀', Ord), ("exists", '∃', Ord), ("nexists", '∄', Ord),
        ("emptyset", '∅', Ord), ("varnothing", '∅', Ord), ("hbar", 'ℏ', Ord), ("hslash", 'ℏ', Ord), ("ell", 'ℓ', Ord), ("Re", 'ℜ', Ord),
        ("Im", 'ℑ', Ord), ("aleph", 'ℵ', Ord), ("beth", 'ℶ', Ord), ("wp", '℘', Ord), ("angle", '∠', Ord), ("measuredangle", '∡', Ord),
        ("triangle", '△', Ord), ("neg", '¬', Ord), ("lnot", '¬', Ord), ("prime", '′', Ord), ("top", '⊤', Ord), ("bot", '⊥', Ord),
        ("checkmark", '✓', Ord), ("dots", '…', Inner), ("ldots", '…', Inner), ("cdots", '⋯', Inner), ("vdots", '⋮', Ord), ("ddots", '⋱', Inner),
        ("dotsc", '…', Inner), ("dotsb", '⋯', Inner), ("degree", '°', Ord), ("circledR", '®', Ord), ("S", '§', Ord), ("P", '¶', Ord),
        ("clubsuit", '♣', Ord), ("diamondsuit", '♢', Ord), ("heartsuit", '♡', Ord), ("spadesuit", '♠', Ord), ("flat", '♭', Ord),
        ("natural", '♮', Ord), ("sharp", '♯', Ord), ("square", '□', Ord), ("Box", '□', Ord), ("blacksquare", '■', Ord),
        ("lozenge", '◊', Ord), ("surd", '√', Ord), ("imath", 'ı', Ord), ("jmath", 'ȷ', Ord), ("complement", '∁', Ord),
        ("therefore", '∴', Rel), ("because", '∵', Rel), ("colon", ':', Punct), ("vert", '|', Ord), ("Vert", '‖', Ord),
        ("backslash", '\\', Ord), ("langle", '⟨', Open), ("rangle", '⟩', Close), ("lfloor", '⌊', Open), ("rfloor", '⌋', Close),
        ("lceil", '⌈', Open), ("rceil", '⌉', Close), ("lvert", '|', Open), ("rvert", '|', Close), ("lVert", '‖', Open), ("rVert", '‖', Close),
        ("ldotp", '.', Punct), ("cdotp", '⋅', Punct), ("lbrack", '[', Open), ("rbrack", ']', Close), ("textbackslash", '\\', Ord),
        ("euro", '€', Ord), ("pounds", '£', Ord), ("yen", '¥', Ord), ("copyright", '©', Ord), ("dag", '†', Ord), ("ddag", '‡', Ord),
    ]
};
