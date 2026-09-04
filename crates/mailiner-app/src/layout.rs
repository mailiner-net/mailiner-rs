//! Mail chrome layout: persisted pane sizes applied as CSS variables on `#app`.

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
}
