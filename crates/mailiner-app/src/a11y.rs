//! Shared accessibility helpers: skip-link target, focus restore, contrast.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

/// Fragment target for the mail-chrome skip link (`#messageview`).
pub const SKIP_TO_MESSAGE_ID: &str = "messageview";

/// WCAG 2.x AA contrast for normal text.
pub const WCAG_AA_NORMAL: f64 = 4.5;

/// WCAG 2.x AA contrast for UI components / large text.
pub const WCAG_AA_UI: f64 = 3.0;

/// Capture the focused element and restore it when the caller unmounts.
///
/// Mount this only inside an overlay that is not in the tree when closed.
#[component]
pub fn RestoreFocus() -> Element {
    use_restore_focus_on_unmount();
    rsx! {}
}

/// Remember `document.activeElement` now; focus it again on drop.
pub fn use_restore_focus_on_unmount() {
    let saved = use_hook(|| Rc::new(RefCell::new(capture_active_element())));
    use_drop(move || restore_focus(saved.borrow().clone()));
}

/// Move keyboard focus to an element by id (skip link, overlay close).
pub fn focus_element_by_id(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(id))
        else {
            return;
        };
        if let Ok(html) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = html.focus();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = id;
    }
}

fn capture_active_element() -> Option<web_sys::HtmlElement> {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        web_sys::window()?
            .document()?
            .active_element()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

fn restore_focus(el: Option<web_sys::HtmlElement>) {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(el) = el {
            let _ = el.focus();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = el;
    }
}

/// sRGB hex (`#rgb` / `#rrggbb`) to relative luminance (WCAG 2.x).
pub fn relative_luminance(hex: &str) -> Option<f64> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    Some(0.2126 * srgb_channel(r) + 0.7152 * srgb_channel(g) + 0.0722 * srgb_channel(b))
}

/// Contrast ratio of two sRGB hex colors (lighter / darker), 1–21.
pub fn contrast_ratio(a: &str, b: &str) -> Option<f64> {
    let la = relative_luminance(a)?;
    let lb = relative_luminance(b)?;
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    Some((hi + 0.05) / (lo + 0.05))
}

fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let s = hex.strip_prefix('#')?;
    match s.len() {
        3 => {
            let r = u8::from_str_radix(&s[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&s[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&s[2..3].repeat(2), 16).ok()?;
            Some((r, g, b))
        }
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some((r, g, b))
        }
        _ => None,
    }
}

fn srgb_channel(c: u8) -> f64 {
    let s = f64::from(c) / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_target_matches_message_view_id() {
        assert_eq!(SKIP_TO_MESSAGE_ID, "messageview");
    }

    #[test]
    fn contrast_black_on_white_is_21() {
        let ratio = contrast_ratio("#000", "#fff").unwrap();
        assert!((ratio - 21.0).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn contrast_same_color_is_1() {
        let ratio = contrast_ratio("#0a0a0a", "#0a0a0a").unwrap();
        assert!((ratio - 1.0).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(contrast_ratio("red", "#fff").is_none());
        assert!(contrast_ratio("#gg0000", "#fff").is_none());
    }

    // Tokens must stay in sync with `:root` / dark theme in `assets/main.css`.
    #[test]
    fn light_theme_text_meets_aa() {
        let bg = "#ffffff";
        assert!(contrast_ratio("#0a0a0a", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#5f6368", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#6b6b6b", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#0066cc", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#b71c1c", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#0066cc", bg).unwrap() >= WCAG_AA_UI);
    }

    #[test]
    fn dark_theme_text_meets_aa() {
        let bg = "#121212";
        assert!(contrast_ratio("#e8e8e8", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#9aa0a6", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#888888", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#4da3ff", bg).unwrap() >= WCAG_AA_NORMAL);
        assert!(contrast_ratio("#ef9a9a", bg).unwrap() >= WCAG_AA_NORMAL);
    }

    #[test]
    fn former_light_subtle_gray_failed_aa() {
        let ratio = contrast_ratio("#888888", "#ffffff").unwrap();
        assert!(ratio < WCAG_AA_NORMAL, "{ratio}");
    }
}
