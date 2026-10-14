//! A small XML reader for Office documents: elements with their prefixed
//! names (`w:p`), attributes and text, as a tree. No DTDs or namespaces
//! resolution: Office writes its prefixes the same way everywhere.

#[derive(Debug, Default, Clone)]
pub struct Element {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone)]
pub enum Node {
    Element(Element),
    Text(String),
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// The child elements, in order.
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        })
    }

    /// The first child element named `name`.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }

    /// The first element named `name` anywhere below, depth first.
    pub fn find(&self, name: &str) -> Option<&Element> {
        self.elements().find_map(|e| if e.name == name { Some(e) } else { e.find(name) })
    }

    /// Every element named `name` below, depth first, not looking inside
    /// those found.
    pub fn find_all<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for e in self.elements() {
            if e.name == name {
                out.push(e);
            } else {
                e.find_all(name, out);
            }
        }
    }

    /// All the text below, joined.
    pub fn text(&self) -> String {
        let mut out = String::new();
        fn walk(e: &Element, out: &mut String) {
            for n in &e.children {
                match n {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(c) => walk(c, out),
                }
            }
        }
        walk(self, &mut out);
        out
    }
}

/// The document's root element. Bad markup ends the tree where it is found
/// rather than failing: a document's text is still worth having.
pub fn parse(bytes: &[u8]) -> Element {
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes));
    let s = text.as_ref();
    let mut stack: Vec<Element> = vec![Element::default()];
    let mut i = 0;
    while i < s.len() {
        let Some(lt) = s[i..].find('<').map(|k| i + k) else {
            push_text(&mut stack, &s[i..]);
            break;
        };
        if lt > i {
            push_text(&mut stack, &s[i..lt]);
        }
        let rest = &s[lt..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(s.len(), |k| lt + k + 3);
        } else if let Some(cdata) = rest.strip_prefix("<![CDATA[") {
            let end = cdata.find("]]>").unwrap_or(cdata.len());
            if let Some(top) = stack.last_mut() {
                top.children.push(Node::Text(cdata[..end].to_string()));
            }
            i = lt + 9 + end + 3;
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            i = rest.find('>').map_or(s.len(), |k| lt + k + 1);
        } else if let Some(close) = rest.strip_prefix("</") {
            let end = close.find('>').unwrap_or(close.len());
            let name = close[..end].trim();
            // Close up to the matching element (ill-nested markup closes
            // what is inside it).
            if let Some(depth) = stack.iter().rposition(|e| e.name == name).filter(|&d| d > 0) {
                while stack.len() > depth {
                    let done = stack.pop().expect("deeper than the root");
                    stack.last_mut().expect("the root").children.push(Node::Element(done));
                }
            }
            i = lt + 2 + end + 1;
        } else {
            let Some(end) = tag_end(rest) else { break };
            let inner = &rest[1..end];
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let (name, attrs) = name_and_attrs(inner);
            let element = Element { name, attrs, children: Vec::new() };
            if empty {
                stack.last_mut().expect("the root").children.push(Node::Element(element));
            } else {
                stack.push(element);
            }
            i = lt + end + 1;
        }
    }
    while stack.len() > 1 {
        let done = stack.pop().expect("deeper than the root");
        stack.last_mut().expect("the root").children.push(Node::Element(done));
    }
    let root = stack.pop().expect("the root");
    root.children.into_iter().find_map(|n| match n {
        Node::Element(e) => Some(e),
        Node::Text(_) => None,
    })
    .unwrap_or_default()
}

fn push_text(stack: &mut [Element], raw: &str) {
    if let Some(top) = stack.last_mut() {
        top.children.push(Node::Text(unescape(raw)));
    }
}

/// Where a tag ends: its `>`, outside quoted attribute values.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

fn name_and_attrs(inner: &str) -> (String, Vec<(String, String)>) {
    let inner = inner.trim();
    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let name = inner[..name_end].to_string();
    let mut attrs = Vec::new();
    let mut rest = &inner[name_end..];
    while let Some(eq) = rest.find('=') {
        let key = rest[..eq].trim().to_string();
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
        let Some(close) = after[1..].find(q) else { break };
        attrs.push((key, unescape(&after[1..1 + close])));
        rest = &after[1 + close + 1..];
    }
    (name, attrs)
}

/// Replaces XML's character references: the five named ones and numbers.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let Some(semi) = after.find(';').filter(|&k| k <= 10) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = &after[..semi];
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_tree_with_attributes_and_text() {
        let doc = parse(br#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body><w:p><w:r><w:t xml:space="preserve">a &amp; b </w:t></w:r><w:r><w:br/></w:r></w:p><!-- note --><w:p w:x='1'>&#x41;&#66;</w:p></w:body></w:document>"#);
        assert_eq!(doc.name, "w:document");
        let body = doc.child("w:body").unwrap();
        let paras: Vec<&Element> = body.elements().collect();
        assert_eq!(paras.len(), 2);
        assert_eq!(paras[0].text(), "a & b ");
        assert!(paras[0].find("w:br").is_some());
        assert_eq!(paras[1].attr("w:x"), Some("1"));
        assert_eq!(paras[1].text(), "AB");
    }

    #[test]
    fn bad_markup_still_gives_its_text() {
        let doc = parse(b"<a><b>one<c>two</b>three</a>");
        assert_eq!(doc.text(), "onetwothree");
        let _ = parse(b"<a attr=\"never closed");
        let _ = parse(b"");
        let _ = parse(b"</x></y>");
    }
}
