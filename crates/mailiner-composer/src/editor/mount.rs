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
    /// moves the caret out of it. A non-collapsed selection is wrapped.
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
            wrap_range(&sel, &range, tag)
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
        if sel.remove_all_ranges().is_err() {
            return false;
        }
        sel.add_range(&range).is_ok()
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
