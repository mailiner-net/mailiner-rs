//! Drop collapsed rich-text caret markers on export.
//!
//! Bold, italic, and underline at a collapsed caret insert `<tag>\u{200b}</tag>`
//! so the browser keeps the element. [`crate::sanitize_for_edit`] must leave
//! that character in place: it runs on every editor sync and would delete the
//! live caret. Export removes a `strong`/`b`/`em`/`i`/`u` element whose text
//! is only U+200B, and a leading U+200B inside those tags. A U+200B anywhere
//! else (including a pasted word-break hint) stays.

use std::ops::Range;

const ZWSP: char = '\u{200b}';

pub(crate) fn strip_collapsed_format_markers(html: String) -> String {
    if !mentions_zwsp(&html) {
        return html;
    }
    let Some(mut nodes) = parse_fragment(&html) else {
        return html;
    };
    clean_children(&html, &mut nodes);
    let mut out = String::with_capacity(html.len());
    emit(&mut out, &html, &nodes);
    out
}

fn mentions_zwsp(html: &str) -> bool {
    if html.contains(ZWSP) {
        return true;
    }
    let mut rest = html;
    while let Some(at) = rest.find("&#") {
        rest = &rest[at..];
        if starts_with_zwsp_entity(rest) {
            return true;
        }
        rest = &rest[1..];
    }
    false
}

fn starts_with_zwsp_entity(raw: &str) -> bool {
    (raw.len() >= 7 && raw[..7].eq_ignore_ascii_case("&#8203;"))
        || (raw.len() >= 8 && raw[..8].eq_ignore_ascii_case("&#x200b;"))
}

fn trim_leading_zwsp(raw: &str) -> &str {
    let mut rest = raw;
    loop {
        if let Some(stripped) = rest.strip_prefix(ZWSP) {
            rest = stripped;
            continue;
        }
        if raw_starts_with_entity(rest, "&#8203;") {
            rest = &rest[7..];
            continue;
        }
        if raw_starts_with_entity(rest, "&#x200b;") {
            rest = &rest[8..];
            continue;
        }
        return rest;
    }
}

fn raw_starts_with_entity(raw: &str, entity: &str) -> bool {
    raw.len() >= entity.len() && raw[..entity.len()].eq_ignore_ascii_case(entity)
}

fn is_format_tag(tag: &str) -> bool {
    matches!(tag, "strong" | "b" | "em" | "i" | "u")
}

fn is_void_tag(tag: &str) -> bool {
    matches!(
        tag,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

struct Node {
    span: Range<usize>,
    kind: NodeKind,
}

enum NodeKind {
    Text,
    Element {
        tag: String,
        /// Byte index just after the opening tag's `>`.
        open_end: usize,
        /// Byte index of the closing tag's `<`, or `open_end` for a void tag.
        close_start: usize,
        void: bool,
        children: Vec<Node>,
    },
}

struct Frame {
    tag: String,
    open_start: usize,
    open_end: usize,
    children: Vec<Node>,
}

struct OpenTag {
    tag: String,
    open_start: usize,
    open_end: usize,
    void: bool,
}

fn parse_fragment(html: &str) -> Option<Vec<Node>> {
    let bytes = html.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut root = Vec::new();
    let mut i = 0;
    while i < html.len() {
        if bytes[i] != b'<' {
            let start = i;
            i += 1;
            while i < html.len() && bytes[i] != b'<' {
                i += 1;
            }
            push_node(
                &mut stack,
                &mut root,
                Node {
                    span: start..i,
                    kind: NodeKind::Text,
                },
            );
            continue;
        }
        if html[i..].starts_with("</") {
            let close_start = i;
            i += 2;
            let name_start = i;
            while i < html.len() && bytes[i].is_ascii_alphanumeric() {
                i += 1;
            }
            if name_start == i {
                return None;
            }
            let name = html[name_start..i].to_ascii_lowercase();
            while i < html.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= html.len() || bytes[i] != b'>' {
                return None;
            }
            i += 1;
            let frame = stack.pop()?;
            if frame.tag != name {
                return None;
            }
            push_node(
                &mut stack,
                &mut root,
                Node {
                    span: frame.open_start..i,
                    kind: NodeKind::Element {
                        tag: frame.tag,
                        open_end: frame.open_end,
                        close_start,
                        void: false,
                        children: frame.children,
                    },
                },
            );
            continue;
        }
        if html[i..].starts_with("<!") || html[i..].starts_with("<?") {
            return None;
        }
        let open = parse_open_tag(html, &mut i)?;
        if open.void {
            push_node(
                &mut stack,
                &mut root,
                Node {
                    span: open.open_start..open.open_end,
                    kind: NodeKind::Element {
                        tag: open.tag,
                        open_end: open.open_end,
                        close_start: open.open_end,
                        void: true,
                        children: Vec::new(),
                    },
                },
            );
        } else {
            stack.push(Frame {
                tag: open.tag,
                open_start: open.open_start,
                open_end: open.open_end,
                children: Vec::new(),
            });
        }
    }
    if stack.is_empty() {
        Some(root)
    } else {
        None
    }
}

fn parse_open_tag(html: &str, i: &mut usize) -> Option<OpenTag> {
    let bytes = html.as_bytes();
    let open_start = *i;
    if *i >= html.len() || bytes[*i] != b'<' {
        return None;
    }
    *i += 1;
    let name_start = *i;
    while *i < html.len() && bytes[*i].is_ascii_alphanumeric() {
        *i += 1;
    }
    if name_start == *i {
        return None;
    }
    let tag = html[name_start..*i].to_ascii_lowercase();
    let mut quote = None;
    let mut self_close = false;
    while *i < html.len() {
        let c = bytes[*i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            *i += 1;
            continue;
        }
        match c {
            b'"' | b'\'' => {
                quote = Some(c);
                *i += 1;
            }
            b'/' => {
                self_close = true;
                *i += 1;
            }
            b'>' => {
                *i += 1;
                return Some(OpenTag {
                    void: self_close || is_void_tag(&tag),
                    tag,
                    open_start,
                    open_end: *i,
                });
            }
            _ => *i += 1,
        }
    }
    None
}

fn push_node(stack: &mut [Frame], root: &mut Vec<Node>, node: Node) {
    if let Some(frame) = stack.last_mut() {
        frame.children.push(node);
    } else {
        root.push(node);
    }
}

fn clean_children(html: &str, children: &mut Vec<Node>) {
    for child in children.iter_mut() {
        if let NodeKind::Element {
            tag,
            children: inner,
            ..
        } = &mut child.kind
        {
            clean_children(html, inner);
            if is_format_tag(tag) {
                strip_leading_zwsp_nodes(html, inner);
            }
        }
    }
    children.retain(|child| !is_content_free_format(html, child));
}

fn strip_leading_zwsp_nodes(html: &str, children: &mut Vec<Node>) {
    while let Some(first) = children.first_mut() {
        if !matches!(first.kind, NodeKind::Text) {
            break;
        }
        let raw = &html[first.span.start..first.span.end];
        let trimmed = trim_leading_zwsp(raw);
        if trimmed.len() == raw.len() {
            break;
        }
        first.span.start += raw.len() - trimmed.len();
        if first.span.is_empty() {
            children.remove(0);
            continue;
        }
        break;
    }
}

/// A formatting element whose remaining text is empty or only U+200B.
///
/// Nested marker-only wrappers are removed first, so an outer tag that held
/// only those wrappers is content-free too and is removed here.
fn is_content_free_format(html: &str, node: &Node) -> bool {
    let NodeKind::Element { tag, children, .. } = &node.kind else {
        return false;
    };
    if !is_format_tag(tag) {
        return false;
    }
    !children.iter().any(|child| match &child.kind {
        NodeKind::Text => {
            let raw = &html[child.span.start..child.span.end];
            !raw.is_empty() && !trim_leading_zwsp(raw).is_empty()
        }
        NodeKind::Element { .. } => true,
    })
}

fn emit(out: &mut String, html: &str, nodes: &[Node]) {
    for node in nodes {
        match &node.kind {
            NodeKind::Text => out.push_str(&html[node.span.start..node.span.end]),
            NodeKind::Element {
                open_end,
                close_start,
                void,
                children,
                ..
            } => {
                out.push_str(&html[node.span.start..*open_end]);
                emit(out, html, children);
                if !void {
                    out.push_str(&html[*close_start..node.span.end]);
                }
            }
        }
    }
}
