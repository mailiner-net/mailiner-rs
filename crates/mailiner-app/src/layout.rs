//! Mail chrome layout: persisted pane sizes applied as CSS variables on `#app`.

/// CSS `max-width` (px) at or below which mail chrome is one pane at a time.
///
/// Covers phones and typical tablet portrait (iPad 768). At 1024px and up the
/// desktop stacked / classic chrome stays in place.
pub const SINGLE_PANE_MAX_WIDTH_PX: f64 = 1023.0;

/// Which mail pane is full-screen on a narrow viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MobilePane {
    /// Folder tree.
    #[default]
    Folders,
    /// Message list for the selected folder.
    List,
    /// Open message.
    Viewer,
}

impl MobilePane {
    /// Class on `#app` so narrow-viewport CSS can hide the other panes.
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Folders => "pane-folders",
            Self::List => "pane-list",
            Self::Viewer => "pane-viewer",
        }
    }

    /// One step toward the folder tree (viewer → list → folders).
    pub fn back(self) -> Self {
        match self {
            Self::Viewer => Self::List,
            Self::List | Self::Folders => Self::Folders,
        }
    }

    /// Opening a folder shows the message list, even if a row is auto-selected.
    pub fn after_select_mailbox() -> Self {
        Self::List
    }

    /// Opening a message (tap / keyboard) shows the viewer.
    pub fn after_select_message() -> Self {
        Self::Viewer
    }
}

/// `true` when `width_px` should use the single-pane chrome.
pub fn is_single_pane_width(width_px: f64) -> bool {
    width_px > 0.0 && width_px <= SINGLE_PANE_MAX_WIDTH_PX
}

pub const FOLDER_WIDTH_KEY: &str = "mailiner.layout.folderWidthPx";
pub const LIST_HEIGHT_KEY: &str = "mailiner.layout.listHeightPct";
pub const LIST_WIDTH_KEY: &str = "mailiner.layout.listWidthPx";

pub const FOLDER_WIDTH_DEFAULT: f64 = 240.0;
pub const FOLDER_WIDTH_MIN: f64 = 160.0;
pub const FOLDER_WIDTH_MAX: f64 = 480.0;

pub const LIST_HEIGHT_DEFAULT: f64 = 40.0;
pub const LIST_HEIGHT_MIN: f64 = 18.0;
pub const LIST_HEIGHT_MAX: f64 = 70.0;

pub const LIST_WIDTH_DEFAULT: f64 = 340.0;
pub const LIST_WIDTH_MIN: f64 = 240.0;
pub const LIST_WIDTH_MAX: f64 = 520.0;

pub fn clamp_folder_width_px(px: f64) -> f64 {
    px.clamp(FOLDER_WIDTH_MIN, FOLDER_WIDTH_MAX)
}

pub fn clamp_list_height_pct(pct: f64) -> f64 {
    pct.clamp(LIST_HEIGHT_MIN, LIST_HEIGHT_MAX)
}

pub fn clamp_list_width_px(px: f64) -> f64 {
    px.clamp(LIST_WIDTH_MIN, LIST_WIDTH_MAX)
}

/// Read saved sizes and set pane CSS variables on `#app`.
pub fn apply_saved_layout() {
    #[cfg(target_arch = "wasm32")]
    wasm::apply_saved();
}

pub fn set_folder_width_px(px: f64) {
    #[cfg(target_arch = "wasm32")]
    wasm::set_folder_width_px(px);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = px;
}

pub fn set_list_height_pct(pct: f64) {
    #[cfg(target_arch = "wasm32")]
    wasm::set_list_height_pct(pct);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = pct;
}

pub fn set_list_width_px(px: f64) {
    #[cfg(target_arch = "wasm32")]
    wasm::set_list_width_px(px);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = px;
}

pub fn persist_layout() {
    #[cfg(target_arch = "wasm32")]
    wasm::persist();
}

pub fn reset_folder_width() {
    set_folder_width_px(FOLDER_WIDTH_DEFAULT);
    persist_layout();
}

pub fn reset_list_height() {
    set_list_height_pct(LIST_HEIGHT_DEFAULT);
    persist_layout();
}

pub fn reset_list_width() {
    set_list_width_px(LIST_WIDTH_DEFAULT);
    persist_layout();
}

/// Clear persisted pane sizes. Safe from settings (no `#app` chrome).
pub fn reset_saved_layout() {
    #[cfg(target_arch = "wasm32")]
    wasm::clear_saved();
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use wasm_bindgen::JsCast;
    use web_sys::HtmlElement;

    fn app_style() -> Option<web_sys::CssStyleDeclaration> {
        let document = web_sys::window()?.document()?;
        let app = document.get_element_by_id("app")?;
        Some(app.unchecked_ref::<HtmlElement>().style())
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }

    fn load_f64(key: &str) -> Option<f64> {
        storage()?
            .get_item(key)
            .ok()
            .flatten()
            .and_then(|s| s.parse().ok())
    }

    pub fn apply_saved() {
        let folder = load_f64(FOLDER_WIDTH_KEY)
            .map(clamp_folder_width_px)
            .unwrap_or(FOLDER_WIDTH_DEFAULT);
        let list_h = load_f64(LIST_HEIGHT_KEY)
            .map(clamp_list_height_pct)
            .unwrap_or(LIST_HEIGHT_DEFAULT);
        let list_w = load_f64(LIST_WIDTH_KEY)
            .map(clamp_list_width_px)
            .unwrap_or(LIST_WIDTH_DEFAULT);
        set_folder_width_px(folder);
        set_list_height_pct(list_h);
        set_list_width_px(list_w);
    }

    pub fn set_folder_width_px(px: f64) {
        let px = clamp_folder_width_px(px);
        if let Some(style) = app_style() {
            let _ = style.set_property("--folder-pane-width", &format!("{px:.0}px"));
        }
    }

    pub fn set_list_height_pct(pct: f64) {
        let pct = clamp_list_height_pct(pct);
        if let Some(style) = app_style() {
            let _ = style.set_property("--message-list-height", &format!("{pct:.1}%"));
        }
    }

    pub fn set_list_width_px(px: f64) {
        let px = clamp_list_width_px(px);
        if let Some(style) = app_style() {
            let _ = style.set_property("--message-list-width", &format!("{px:.0}px"));
        }
    }

    pub fn clear_saved() {
        if let Some(storage) = storage() {
            let _ = storage.remove_item(FOLDER_WIDTH_KEY);
            let _ = storage.remove_item(LIST_HEIGHT_KEY);
            let _ = storage.remove_item(LIST_WIDTH_KEY);
        }
        set_folder_width_px(FOLDER_WIDTH_DEFAULT);
        set_list_height_pct(LIST_HEIGHT_DEFAULT);
        set_list_width_px(LIST_WIDTH_DEFAULT);
    }

    pub fn persist() {
        let Some(storage) = storage() else {
            return;
        };
        let Some(style) = app_style() else {
            return;
        };
        if let Ok(w) = style.get_property_value("--folder-pane-width") {
            let trimmed = w.trim().trim_end_matches("px");
            if trimmed.parse::<f64>().is_ok() {
                let _ = storage.set_item(FOLDER_WIDTH_KEY, trimmed);
            }
        }
        if let Ok(h) = style.get_property_value("--message-list-height") {
            let trimmed = h.trim().trim_end_matches('%');
            if trimmed.parse::<f64>().is_ok() {
                let _ = storage.set_item(LIST_HEIGHT_KEY, trimmed);
            }
        }
        if let Ok(w) = style.get_property_value("--message-list-width") {
            let trimmed = w.trim().trim_end_matches("px");
            if trimmed.parse::<f64>().is_ok() {
                let _ = storage.set_item(LIST_WIDTH_KEY, trimmed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_folder_width() {
        assert_eq!(clamp_folder_width_px(80.0), FOLDER_WIDTH_MIN);
        assert_eq!(clamp_folder_width_px(999.0), FOLDER_WIDTH_MAX);
        assert_eq!(clamp_folder_width_px(200.0), 200.0);
    }

    #[test]
    fn clamps_list_height() {
        assert_eq!(clamp_list_height_pct(5.0), LIST_HEIGHT_MIN);
        assert_eq!(clamp_list_height_pct(90.0), LIST_HEIGHT_MAX);
        assert_eq!(clamp_list_height_pct(40.0), 40.0);
    }

    #[test]
    fn clamps_list_width() {
        assert_eq!(clamp_list_width_px(80.0), LIST_WIDTH_MIN);
        assert_eq!(clamp_list_width_px(999.0), LIST_WIDTH_MAX);
        assert_eq!(clamp_list_width_px(340.0), 340.0);
    }

    #[test]
    fn single_pane_covers_phone_and_tablet_portrait() {
        assert!(is_single_pane_width(390.0));
        assert!(is_single_pane_width(430.0));
        assert!(is_single_pane_width(768.0));
        assert!(is_single_pane_width(820.0));
        assert!(is_single_pane_width(SINGLE_PANE_MAX_WIDTH_PX));
        assert!(!is_single_pane_width(1024.0));
        assert!(!is_single_pane_width(1280.0));
        assert!(!is_single_pane_width(0.0));
    }

    #[test]
    fn mobile_pane_classes_and_back() {
        assert_eq!(MobilePane::Folders.css_class(), "pane-folders");
        assert_eq!(MobilePane::List.css_class(), "pane-list");
        assert_eq!(MobilePane::Viewer.css_class(), "pane-viewer");
        assert_eq!(MobilePane::Viewer.back(), MobilePane::List);
        assert_eq!(MobilePane::List.back(), MobilePane::Folders);
        assert_eq!(MobilePane::Folders.back(), MobilePane::Folders);
        assert_eq!(MobilePane::after_select_mailbox(), MobilePane::List);
        assert_eq!(MobilePane::after_select_message(), MobilePane::Viewer);
    }
}
