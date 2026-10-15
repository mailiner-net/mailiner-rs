//! Mount helpers for the shadow-hosted editor.
//!
//! The contenteditable host must set `spellcheck` to [`super::SPELLCHECK`].
//! Compose CSS stays on the light-DOM host; editor CSS lives in the open shadow
//! root so it cannot restyle Mailiner chrome (and vice versa).

use super::commands::EditorCommand;
use super::rich::{html_for_edit, EMPTY_EDIT_HTML};
use crate::model::InlineImage;

/// Light-DOM host id (`#mailiner-compose-editor`).
pub const EDITOR_HOST_ID: &str = "mailiner-compose-editor";

/// Contenteditable id inside the shadow root.
pub const EDITOR_INNER_ID: &str = "mlnr-compose-editable";

/// Shadow-root styles. Color inherits from the host so dark/light chrome apply.
pub const EDITOR_SHADOW_CSS: &str = r#"
:host { display: block; min-height: 12rem; color-scheme: inherit; background: transparent; color: inherit; }
#mlnr-compose-editable {
  outline: none;
  min-height: 12rem;
  font: inherit;
  line-height: 1.45;
  overflow-wrap: anywhere;
  color: inherit;
}
#mlnr-compose-editable:focus { outline: none; }
blockquote {
  margin: 0.5em 0;
  padding: 0 0 0 0.75em;
  border-left: 3px solid currentColor;
  opacity: 0.9;
}
.mlnr-compose-quote { margin-top: 1em; opacity: 0.88; }
.mlnr-attribution { font-size: 0.95em; opacity: 0.85; }
img { max-width: 100%; height: auto; }
a { color: inherit; text-decoration: underline; }
h2 { font-size: 1.15em; margin: 0.6em 0 0.3em; }
p { margin: 0 0 0.6em; }
"#;

/// Inject sanitized `html` into an open-shadow contenteditable on `host_id`.
pub fn mount_editor(host_id: &str, html: &str, images: &[InlineImage]) {
    mount_editor_html(host_id, &html_for_edit(html, images));
}

/// Inject already-sanitized edit HTML (no further cid rewrite).
pub fn mount_editor_html(host_id: &str, sanitized_html: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    web::mount_editor_html(host_id, sanitized_html);
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = (host_id, sanitized_html);
    }
}

/// Read the live editor HTML and sanitize it for draft storage.
pub fn read_editor_html(host_id: &str) -> Option<String> {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web::read_raw_html(host_id).map(|raw| super::rich::html_from_editor(&raw))
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = host_id;
        None
    }
}

/// Replace the editor contents without rebuilding the shadow tree.
pub fn set_editor_html(host_id: &str, html: &str, images: &[InlineImage]) {
    mount_editor(host_id, html, images);
}

/// Focus the contenteditable (keeps the selection for toolbar commands).
pub fn focus_editor(host_id: &str) -> bool {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web::focus_editor(host_id)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = host_id;
        false
    }
}

/// Run `command` against the focused editor selection.
pub fn exec_editor_command(host_id: &str, command: EditorCommand, link_href: Option<&str>) -> bool {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web::exec_editor_command(host_id, command, link_href)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = (host_id, command, link_href);
        false
    }
}

/// Insert an HTML fragment at the caret (already sanitized by the caller).
pub fn insert_editor_html(host_id: &str, html: &str) -> bool {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web::insert_html(host_id, html)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = (host_id, html);
        false
    }
}

/// Enable or disable editing.
pub fn set_editor_enabled(host_id: &str, enabled: bool) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    web::set_enabled(host_id, enabled);
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = (host_id, enabled);
    }
}

/// Prompt for a link URL and normalize it (WASM only; `None` on native).
pub fn prompt_link_href() -> Option<String> {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web::prompt_link_href()
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        None
    }
}

/// Empty placeholder used when mounting a blank editor.
pub fn empty_editor_html() -> &'static str {
    EMPTY_EDIT_HTML
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web {
    use super::{EditorCommand, EDITOR_INNER_ID, EDITOR_SHADOW_CSS};
    use crate::editor::commands::normalize_link_href;
    use crate::editor::SPELLCHECK;
    use crate::sanitize::sanitize_for_edit;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::JsCast;
    use web_sys::{
        Element, HtmlDocument, HtmlElement, Node, Selection, ShadowRoot, ShadowRootInit,
        ShadowRootMode,
    };

    /// `document.execCommand` does not apply inside an open shadow root.
    /// `ShadowRoot.getSelection` is the selection the caret actually uses.
    /// A local extension type keeps the method off `web_sys::ShadowRoot`.
    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(extends = ShadowRoot, js_name = ShadowRoot)]
        type ShadowRootSelection;

        #[wasm_bindgen(catch, method, js_name = getSelection)]
        fn get_selection(
            this: &ShadowRootSelection,
        ) -> Result<Option<Selection>, wasm_bindgen::JsValue>;
    }

    fn shadow_get_selection(shadow: &ShadowRoot) -> Option<Selection> {
        let ext: &ShadowRootSelection = shadow.unchecked_ref();
        ext.get_selection().ok().flatten()
    }

    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    fn html_document() -> Option<HtmlDocument> {
        document()?.dyn_into::<HtmlDocument>().ok()
    }

    fn exec_cmd(command: &str, value: &str) -> bool {
        let Some(doc) = html_document() else {
            return false;
        };
        doc.exec_command_with_show_ui_and_value(command, false, value)
            .unwrap_or(false)
    }

    fn host_el(host_id: &str) -> Option<web_sys::Element> {
        document()?.get_element_by_id(host_id)
    }

    fn ensure_shadow(host: &web_sys::Element) -> Option<ShadowRoot> {
        if let Some(existing) = host.shadow_root() {
            return Some(existing);
        }
        let init = ShadowRootInit::new(ShadowRootMode::Open);
        host.attach_shadow(&init).ok()
    }

    fn editable(shadow: &ShadowRoot) -> Option<HtmlElement> {
        shadow
            .get_element_by_id(EDITOR_INNER_ID)
            .and_then(|el| el.dyn_into::<HtmlElement>().ok())
    }

    pub(super) fn mount_editor_html(host_id: &str, sanitized_html: &str) {
        let Some(document) = document() else {
            return;
        };
        let Some(host) = host_el(host_id) else {
            return;
        };
        let Some(shadow) = ensure_shadow(&host) else {
            return;
        };
        if editable(&shadow).is_none() {
            shadow.set_inner_html("");
            if let Ok(style) = document.create_element("style") {
                style.set_text_content(Some(EDITOR_SHADOW_CSS));
                let _ = shadow.append_child(&style);
            }
            let Ok(edit) = document.create_element("div") else {
                return;
            };
            edit.set_id(EDITOR_INNER_ID);
            let _ = edit.set_attribute("contenteditable", "true");
            let _ = edit.set_attribute("spellcheck", SPELLCHECK);
            let _ = edit.set_attribute("role", "textbox");
            let _ = edit.set_attribute("aria-multiline", "true");
            let _ = edit.set_attribute("aria-label", "Message");
            attach_paste_sanitizer(&edit);
            let _ = shadow.append_child(&edit);
        }
        if let Some(edit) = editable(&shadow) {
            edit.set_inner_html(sanitized_html);
        }
    }

    fn attach_paste_sanitizer(edit: &web_sys::Element) {
        let closure = Closure::wrap(Box::new(move |evt: web_sys::Event| {
            let Ok(clip) = evt.clone().dyn_into::<web_sys::ClipboardEvent>() else {
                return;
            };
            let Some(dt) = clip.clipboard_data() else {
                return;
            };
            let Ok(html) = dt.get_data("text/html") else {
                return;
            };
            if html.trim().is_empty() {
                return;
            }
            evt.prevent_default();
            let clean = sanitize_for_edit(&html);
            let _ = exec_cmd("insertHTML", &clean);
        }) as Box<dyn FnMut(_)>);
        let _ = edit.add_event_listener_with_callback("paste", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    pub(super) fn read_raw_html(host_id: &str) -> Option<String> {
        let host = host_el(host_id)?;
        let shadow = host.shadow_root()?;
        Some(editable(&shadow)?.inner_html())
    }

    pub(super) fn focus_editor(host_id: &str) -> bool {
        let Some(host) = host_el(host_id) else {
            return false;
        };
        let Some(shadow) = host.shadow_root() else {
            return false;
        };
        match editable(&shadow) {
            Some(edit) => edit.focus().is_ok(),
            None => false,
        }
    }

    pub(super) fn set_enabled(host_id: &str, enabled: bool) {
        let Some(host) = host_el(host_id) else {
            return;
        };
        let Some(shadow) = host.shadow_root() else {
            return;
        };
        if let Some(edit) = editable(&shadow) {
            let _ = edit.set_attribute("contenteditable", if enabled { "true" } else { "false" });
        }
    }

    pub(super) fn insert_html(host_id: &str, html: &str) -> bool {
        if !focus_editor(host_id) {
            return false;
        }
        exec_cmd("insertHTML", html)
    }

    pub(super) fn prompt_link_href() -> Option<String> {
        let window = web_sys::window()?;
        let typed = window.prompt_with_message("Link URL").ok().flatten()?;
        normalize_link_href(&typed)
    }

    fn save_selection() -> Option<web_sys::Range> {
        let sel = web_sys::window()?.get_selection().ok().flatten()?;
        if sel.range_count() == 0 {
            return None;
        }
        sel.get_range_at(0).ok()
    }

    fn restore_selection(range: &web_sys::Range) {
        let Some(sel) = web_sys::window().and_then(|w| w.get_selection().ok().flatten()) else {
            return;
        };
        let _ = sel.remove_all_ranges();
        let _ = sel.add_range(range);
    }

    pub(super) fn exec_editor_command(
        host_id: &str,
        command: EditorCommand,
        link_href: Option<&str>,
    ) -> bool {
        if inline_tag(command).is_some() && apply_inline_format(host_id, command) {
            return true;
        }
        let saved = save_selection();
        let owned;
        let value = match command {
            EditorCommand::CreateLink => {
                let href = match link_href {
                    Some(h) => normalize_link_href(h),
                    None => prompt_link_href(),
                };
                owned = match href {
                    Some(h) => h,
                    None => return false,
                };
                owned.as_str()
            }
            other => other.exec_value().unwrap_or(""),
        };
        if !focus_editor(host_id) {
            return false;
        }
        if let Some(range) = saved.as_ref() {
            restore_selection(range);
        }
        exec_cmd(command.exec_name(), value)
    }

    fn inline_tag(command: EditorCommand) -> Option<(&'static str, &'static [&'static str])> {
        match command {
            EditorCommand::Bold => Some(("strong", &["strong", "b"])),
            EditorCommand::Italic => Some(("em", &["em", "i"])),
            EditorCommand::Underline => Some(("u", &["u"])),
            _ => None,
        }
    }

    /// Bold / italic / underline against the shadow-root selection.
    ///
    /// A collapsed caret that is not already inside the mark inserts an empty
    /// wrapper with a zero-width space so the browser keeps the element, and
    /// the caret is placed after that space. A second click on the same mark
    /// moves the caret out of it.
    ///
    /// A selection toggles. When every selected character already sits in the
    /// mark (`strong`/`b`, `em`/`i`, or `u`), that mark is lifted off the
    /// selection, splitting elements the selection covers only in part. Other
    /// inline elements stay. Otherwise the mark is applied to characters that
    /// lack it, without nesting a second copy on text that already has one.
    fn apply_inline_format(host_id: &str, command: EditorCommand) -> bool {
        let Some((tag, aliases)) = inline_tag(command) else {
            return false;
        };
        if !focus_editor(host_id) {
            return false;
        }
        let Some(host) = host_el(host_id) else {
            return false;
        };
        let Some(shadow) = host.shadow_root() else {
            return false;
        };
        let Some(edit) = editable(&shadow) else {
            return false;
        };
        let Some(sel) = selection_in_editor(&shadow, &edit) else {
            return false;
        };
        if sel.range_count() == 0 {
            return false;
        }
        let Ok(range) = sel.get_range_at(0) else {
            return false;
        };
        let applied = if range.collapsed() {
            let inside = range
                .start_container()
                .ok()
                .and_then(|node| formatting_ancestor(&node, aliases, &edit));
            if let Some(existing) = inside {
                place_caret_after(&sel, &existing)
            } else {
                insert_collapsed_wrapper(&sel, &range, tag)
            }
        } else {
            toggle_inline_selection(&sel, &range, tag, aliases, &edit)
        };
        if applied {
            let _ = edit.focus();
        }
        applied
    }

    fn selection_in_editor(shadow: &ShadowRoot, edit: &HtmlElement) -> Option<Selection> {
        let existing = shadow_get_selection(shadow).or_else(|| document_selection_inside(edit));
        if let Some(sel) = existing {
            let inside = sel.range_count() > 0
                && sel
                    .anchor_node()
                    .is_some_and(|node| edit.contains(Some(&node)));
            if inside {
                return Some(sel);
            }
        }
        let sel = shadow_get_selection(shadow)
            .or_else(|| web_sys::window()?.get_selection().ok().flatten())?;
        let document = document()?;
        let range = document.create_range().ok()?;
        if let Some(paragraph) = edit.query_selector("p").ok().flatten() {
            range.set_start(&paragraph, 0).ok()?;
        } else {
            range.set_start(edit, 0).ok()?;
        }
        range.collapse_with_to_start(true);
        sel.remove_all_ranges().ok()?;
        sel.add_range(&range).ok()?;
        Some(sel)
    }

    fn document_selection_inside(edit: &HtmlElement) -> Option<Selection> {
        let sel = web_sys::window()?.get_selection().ok().flatten()?;
        let anchor = sel.anchor_node()?;
        edit.contains(Some(&anchor)).then_some(sel)
    }

    fn formatting_ancestor(node: &Node, tags: &[&str], root: &HtmlElement) -> Option<Element> {
        let root_node: &Node = root;
        let mut current = Some(node.clone());
        while let Some(n) = current {
            if n.is_same_node(Some(root_node)) {
                return None;
            }
            if let Some(el) = n.dyn_ref::<Element>() {
                let name = el.tag_name();
                if tags.iter().any(|tag| name.eq_ignore_ascii_case(tag)) {
                    return Some(el.clone());
                }
            }
            current = n.parent_node();
        }
        None
    }

    fn place_caret_after(sel: &Selection, el: &Element) -> bool {
        let Some(document) = document() else {
            return false;
        };
        let Ok(range) = document.create_range() else {
            return false;
        };
        if range.set_start_after(el).is_err() {
            return false;
        }
        range.collapse_with_to_start(true);
        if sel.remove_all_ranges().is_err() || sel.add_range(&range).is_err() {
            return false;
        }
        // Chrome snaps a caret on an element boundary back into the mark.
        // A text node just outside, with the caret after a zero-width space,
        // is a position it keeps.
        if !caret_inside(sel, el) {
            return true;
        }
        let Some(text) = exit_anchor(&document, el) else {
            return false;
        };
        if range.set_start(&text, 1).is_err() {
            return false;
        }
        range.collapse_with_to_start(true);
        sel.remove_all_ranges().is_ok() && sel.add_range(&range).is_ok()
    }

    fn caret_inside(sel: &Selection, el: &Element) -> bool {
        sel.anchor_node()
            .is_some_and(|node| el.contains(Some(&node)))
    }

    /// Text node immediately after `el` that can hold a caret outside the mark.
    fn exit_anchor(document: &web_sys::Document, el: &Element) -> Option<web_sys::Text> {
        if let Some(next) = el.next_sibling() {
            if let Some(text) = next.dyn_ref::<web_sys::Text>() {
                if text.text_content().as_deref() == Some("\u{200b}") {
                    return Some(text.clone());
                }
            }
        }
        let text = document.create_text_node("\u{200b}");
        el.parent_node()?
            .insert_before(&text, el.next_sibling().as_ref())
            .ok()?;
        Some(text)
    }

    fn insert_collapsed_wrapper(sel: &Selection, range: &web_sys::Range, tag: &str) -> bool {
        let Some(document) = document() else {
            return false;
        };
        let Ok(el) = document.create_element(tag) else {
            return false;
        };
        let text = document.create_text_node("\u{200b}");
        if el.append_child(&text).is_err() {
            return false;
        }
        if range.insert_node(&el).is_err() {
            return false;
        }
        if range.set_start(&text, 1).is_err() {
            return false;
        }
        range.collapse_with_to_start(true);
        if sel.remove_all_ranges().is_err() {
            return false;
        }
        sel.add_range(range).is_ok()
    }

    fn wrap_range(sel: &Selection, range: &web_sys::Range, tag: &str) -> bool {
        let Some(document) = document() else {
            return false;
        };
        let Ok(el) = document.create_element(tag) else {
            return false;
        };
        if range.surround_contents(&el).is_err() {
            let Ok(fragment) = range.extract_contents() else {
                return false;
            };
            if el.append_child(&fragment).is_err() {
                return false;
            }
            if range.insert_node(&el).is_err() {
                return false;
            }
        }
        if sel.remove_all_ranges().is_err() {
            return false;
        }
        if range.select_node_contents(&el).is_err() {
            return false;
        }
        sel.add_range(range).is_ok()
    }

    /// Marker attribute. Empty spans bracket the selection so later splits
    /// cannot invalidate the original range endpoints.
    const MARKER_ATTR: &str = "data-mlnr-fmt";

    struct SelectionMarkers {
        start: Element,
        end: Element,
        live: bool,
    }

    impl SelectionMarkers {
        fn take_down(&mut self) {
            if self.live {
                self.start.remove();
                self.end.remove();
                self.live = false;
            }
        }
    }

    impl Drop for SelectionMarkers {
        fn drop(&mut self) {
            self.take_down();
        }
    }

    fn toggle_inline_selection(
        sel: &Selection,
        range: &web_sys::Range,
        tag: &str,
        aliases: &[&str],
        root: &HtmlElement,
    ) -> bool {
        let Some(mut markers) = insert_markers(range) else {
            return false;
        };
        let handled = if selection_is_formatted(&markers.start, &markers.end, aliases, root) {
            unwrap_marks(&markers.start, &markers.end, aliases, root);
            true
        } else {
            wrap_unformatted(sel, &markers.start, &markers.end, tag, aliases, root)
        };
        if !handled {
            return false;
        }
        let selected = range_between(&markers.start, &markers.end);
        markers.take_down();
        root.normalize();
        remove_empty_inlines(root);
        if let Some(selected) = selected {
            let _ = sel.remove_all_ranges();
            let _ = sel.add_range(&selected);
        }
        true
    }

    fn insert_markers(range: &web_sys::Range) -> Option<SelectionMarkers> {
        let document = document()?;
        let end = document.create_element("span").ok()?;
        end.set_attribute(MARKER_ATTR, "end").ok()?;
        let start = document.create_element("span").ok()?;
        start.set_attribute(MARKER_ATTR, "start").ok()?;
        // Clone both endpoints before either insert. Inserting the end marker
        // first keeps a shared text node from shifting the start offset.
        let end_point = range.clone_range();
        end_point.collapse_with_to_start(false);
        let start_point = range.clone_range();
        start_point.collapse_with_to_start(true);
        if end_point.insert_node(&end).is_err() {
            return None;
        }
        if start_point.insert_node(&start).is_err() {
            end.remove();
            return None;
        }
        Some(SelectionMarkers {
            start,
            end,
            live: true,
        })
    }

    fn range_between(start: &Node, end: &Node) -> Option<web_sys::Range> {
        let document = document()?;
        let range = document.create_range().ok()?;
        range.set_start_after(start).ok()?;
        range.set_end_before(end).ok()?;
        Some(range)
    }

    fn selection_is_formatted(
        start: &Node,
        end: &Node,
        aliases: &[&str],
        root: &HtmlElement,
    ) -> bool {
        let texts = text_nodes_between(root, start, end);
        if !texts.is_empty() {
            return texts
                .iter()
                .all(|node| formatting_ancestor(node, aliases, root).is_some());
        }
        // A selection of only a `<br>` or image is "on" when both edges sit in the mark.
        formatting_ancestor(start, aliases, root).is_some()
            && formatting_ancestor(end, aliases, root).is_some()
    }

    fn text_nodes_between(root: &Node, start: &Node, end: &Node) -> Vec<Node> {
        let Some(sel) = range_between(start, end) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        collect_text(root, &sel, &mut out);
        out
    }

    fn collect_text(node: &Node, sel: &web_sys::Range, out: &mut Vec<Node>) {
        if node.node_type() == Node::TEXT_NODE {
            if text_overlaps(sel, node) {
                out.push(node.clone());
            }
            return;
        }
        for child in child_nodes(node) {
            collect_text(&child, sel, out);
        }
    }

    fn text_overlaps(sel: &web_sys::Range, node: &Node) -> bool {
        let Some(text) = node.dyn_ref::<web_sys::Text>() else {
            return false;
        };
        // `intersectsNode` is false when the node only touches a boundary.
        text.length() > 0 && sel.intersects_node(node).unwrap_or(false)
    }

    fn child_nodes(node: &Node) -> Vec<Node> {
        let mut out = Vec::new();
        let mut current = node.first_child();
        while let Some(child) = current {
            current = child.next_sibling();
            out.push(child);
        }
        out
    }

    fn tag_matches(node: &Node, aliases: &[&str]) -> bool {
        let Some(el) = node.dyn_ref::<Element>() else {
            return false;
        };
        let name = el.tag_name();
        aliases.iter().any(|tag| name.eq_ignore_ascii_case(tag))
    }

    fn unwrap_marks(start: &Element, end: &Element, aliases: &[&str], root: &Element) {
        let matches = collect_matches(root, aliases, start, end);
        for el in matches {
            if el.parent_node().is_none() {
                continue;
            }
            let Some(sel) = range_between(start, end) else {
                return;
            };
            if !sel.intersects_node(&el).unwrap_or(false) {
                continue;
            }
            let start_inside = el.contains(Some(start));
            let end_inside = el.contains(Some(end));
            if start_inside && end_inside {
                split_before(&el, end);
                if el.parent_node().is_some() && el.contains(Some(start)) {
                    if let Some(middle) = split_before(&el, start) {
                        unwrap_element(&middle);
                    }
                }
            } else if end_inside {
                split_before(&el, end);
                if el.parent_node().is_some() {
                    unwrap_element(&el);
                }
            } else if start_inside {
                if let Some(right) = split_before(&el, start) {
                    unwrap_element(&right);
                }
            } else {
                unwrap_element(&el);
            }
        }
    }

    fn collect_matches(root: &Element, aliases: &[&str], start: &Node, end: &Node) -> Vec<Element> {
        let Some(sel) = range_between(start, end) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        collect_matching(root, &sel, aliases, &mut found);
        found.sort_by_key(|el| std::cmp::Reverse(depth_under(el, root)));
        found
    }

    fn collect_matching(
        node: &Node,
        sel: &web_sys::Range,
        aliases: &[&str],
        found: &mut Vec<Element>,
    ) {
        for child in child_nodes(node) {
            if let Some(el) = child.dyn_ref::<Element>().cloned() {
                if tag_matches(&child, aliases) && sel.intersects_node(&child).unwrap_or(false) {
                    found.push(el);
                }
                collect_matching(&child, sel, aliases, found);
            }
        }
    }

    fn depth_under(node: &Node, root: &Node) -> u32 {
        let mut depth = 0;
        let mut current = node.parent_node();
        while let Some(parent) = current {
            depth += 1;
            if parent.is_same_node(Some(root)) {
                break;
            }
            current = parent.parent_node();
        }
        depth
    }

    /// Move `boundary` and everything after it into a new sibling of `el`.
    /// Partially covered ancestors between `boundary` and `el` are split too.
    fn split_before(el: &Element, boundary: &Node) -> Option<Element> {
        if !el.contains(Some(boundary)) || el.is_same_node(Some(boundary)) {
            return None;
        }
        let document = document()?;
        let range = document.create_range().ok()?;
        range.set_start_before(boundary).ok()?;
        let end_offset = u32::try_from(child_nodes(el).len()).ok()?;
        range.set_end(el, end_offset).ok()?;
        if range.collapsed() {
            return None;
        }
        let fragment = range.extract_contents().ok()?;
        let clone = el.clone_node_with_deep(false).ok()?;
        let clone = clone.dyn_into::<Element>().ok()?;
        clone.append_child(&fragment).ok()?;
        let parent = el.parent_node()?;
        parent
            .insert_before(&clone, el.next_sibling().as_ref())
            .ok()?;
        if el.first_child().is_none() {
            el.remove();
        }
        if clone.first_child().is_none() {
            clone.remove();
            return None;
        }
        Some(clone)
    }

    fn unwrap_element(el: &Element) {
        let Some(parent) = el.parent_node() else {
            return;
        };
        while let Some(child) = el.first_child() {
            if parent.insert_before(&child, Some(el)).is_err() {
                return;
            }
        }
        el.remove();
    }

    fn wrap_unformatted(
        sel: &Selection,
        start: &Element,
        end: &Element,
        tag: &str,
        aliases: &[&str],
        root: &HtmlElement,
    ) -> bool {
        let texts = text_nodes_between(root, start, end);
        let mut wrapped = false;
        for node in &texts {
            if formatting_ancestor(node, aliases, root).is_some() {
                continue;
            }
            if wrap_whole(node, tag) {
                wrapped = true;
            }
        }
        if wrapped {
            if let Some(range) = range_between(start, end) {
                merge_adjacent(root, &range, aliases, tag);
            }
            return true;
        }
        // Text was already formatted. The caller treats that as an unwrap.
        if !texts.is_empty() {
            return true;
        }
        let Some(range) = range_between(start, end) else {
            return false;
        };
        wrap_range(sel, &range, tag)
    }

    fn wrap_whole(node: &Node, tag: &str) -> bool {
        let Some(document) = document() else {
            return false;
        };
        let Ok(el) = document.create_element(tag) else {
            return false;
        };
        let Some(parent) = node.parent_node() else {
            return false;
        };
        if parent.insert_before(&el, Some(node)).is_err() {
            return false;
        }
        el.append_child(node).is_ok()
    }

    fn merge_adjacent(root: &Node, sel: &web_sys::Range, aliases: &[&str], canonical: &str) {
        for child in child_nodes(root) {
            if child.node_type() == Node::ELEMENT_NODE {
                merge_adjacent(&child, sel, aliases, canonical);
            }
        }
        let mut child = root.first_child();
        while let Some(current) = child {
            let Some(next) = current.next_sibling() else {
                break;
            };
            if is_plain_alias(&current, aliases) && is_plain_alias(&next, aliases) {
                if let Some(index) = child_index(root, &next) {
                    if point_strictly_inside(sel, root, index) {
                        if let Some(keep) = merge_pair(root, &current, &next, canonical) {
                            child = Some(keep);
                            continue;
                        }
                    }
                }
            }
            child = Some(next);
        }
    }

    fn is_plain_alias(node: &Node, aliases: &[&str]) -> bool {
        let Some(el) = node.dyn_ref::<Element>() else {
            return false;
        };
        tag_matches(node, aliases) && el.get_attribute_names().length() == 0
    }

    fn child_index(parent: &Node, child: &Node) -> Option<u32> {
        let mut index = 0;
        let mut current = parent.first_child();
        while let Some(node) = current {
            if node.is_same_node(Some(child)) {
                return Some(index);
            }
            index += 1;
            current = node.next_sibling();
        }
        None
    }

    /// Chrome reports `-1` when this boundary is before the probe and `1` when it is after.
    /// The selection's own start and end boundaries are not strictly inside.
    fn point_strictly_inside(sel: &web_sys::Range, node: &Node, offset: u32) -> bool {
        let Some(document) = document() else {
            return false;
        };
        let Ok(probe) = document.create_range() else {
            return false;
        };
        if probe.set_start(node, offset).is_err() {
            return false;
        }
        probe.collapse_with_to_start(true);
        let Ok(start_cmp) = sel.compare_boundary_points(web_sys::Range::START_TO_START, &probe)
        else {
            return false;
        };
        let Ok(end_cmp) = sel.compare_boundary_points(web_sys::Range::START_TO_END, &probe) else {
            return false;
        };
        start_cmp == -1 && end_cmp == 1
    }

    fn merge_pair(parent: &Node, current: &Node, next: &Node, canonical: &str) -> Option<Node> {
        let keep = if tag_is(current, canonical) {
            current.clone()
        } else {
            let document = document()?;
            let renamed = document.create_element(canonical).ok()?;
            while let Some(child) = current.first_child() {
                renamed.append_child(&child).ok()?;
            }
            parent.insert_before(&renamed, Some(current)).ok()?;
            remove_node(current);
            renamed.unchecked_into::<Node>()
        };
        while let Some(child) = next.first_child() {
            keep.append_child(&child).ok()?;
        }
        remove_node(next);
        Some(keep)
    }

    fn tag_is(node: &Node, tag: &str) -> bool {
        node.dyn_ref::<Element>()
            .is_some_and(|el| el.tag_name().eq_ignore_ascii_case(tag))
    }

    fn remove_node(node: &Node) {
        if let Some(el) = node.dyn_ref::<Element>() {
            el.remove();
            return;
        }
        if let Some(parent) = node.parent_node() {
            let _ = parent.remove_child(node);
        }
    }

    /// Inline tags a split can leave with no children. `br`, `img`, and `hr`
    /// are omitted: they have no children and are real content.
    const EMPTY_INLINE: &[&str] = &["a", "b", "code", "em", "i", "span", "strong", "u"];

    /// Drop empty inline wrappers. A split clones every ancestor between the
    /// marker and the mark, so unwrapping bold can leave an empty `<em>`
    /// inside the untouched side of a `<strong>`. Removing that `<em>` can
    /// empty the `<strong>`, so this repeats.
    fn remove_empty_inlines(root: &Node) {
        loop {
            let mut empty = Vec::new();
            collect_empty_inlines(root, &mut empty);
            if empty.is_empty() {
                break;
            }
            for el in empty {
                el.remove();
            }
        }
    }

    fn collect_empty_inlines(node: &Node, out: &mut Vec<Element>) {
        for child in child_nodes(node) {
            collect_empty_inlines(&child, out);
            if tag_matches(&child, EMPTY_INLINE) && child.first_child().is_none() {
                if let Some(el) = child.dyn_ref::<Element>() {
                    out.push(el.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_id_is_stable() {
        assert_eq!(EDITOR_HOST_ID, "mailiner-compose-editor");
        assert!(!EDITOR_SHADOW_CSS.is_empty());
    }

    #[test]
    fn native_read_is_none() {
        assert!(read_editor_html(EDITOR_HOST_ID).is_none());
        assert!(!focus_editor(EDITOR_HOST_ID));
        assert!(!exec_editor_command(
            EDITOR_HOST_ID,
            EditorCommand::Bold,
            None
        ));
    }
}
