//! Drag handles between mail chrome panes.

use dioxus::prelude::*;

use crate::layout::{
    clamp_folder_width_px, clamp_list_height_pct, clamp_list_width_px, persist_layout,
    reset_folder_width, reset_list_height, reset_list_width, set_folder_width_px,
    set_list_height_pct, set_list_width_px,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Folder tree vs list/viewer (vertical bar, col-resize).
    Folder,
    /// Message list vs viewer, stacked (horizontal bar, row-resize).
    List,
    /// Message list vs viewer, classic three columns (vertical bar, col-resize).
    ListWidth,
}

#[component]
pub fn SplitHandle(axis: SplitAxis) -> Element {
    let mut dragging = use_signal(|| false);
    let class = match axis {
        SplitAxis::Folder | SplitAxis::ListWidth => "split-handle split-handle-col",
        SplitAxis::List => "split-handle split-handle-row",
    };
    let orientation = match axis {
        SplitAxis::Folder | SplitAxis::ListWidth => "vertical",
        SplitAxis::List => "horizontal",
    };

    let class = if dragging() {
        format!("{class} is-dragging")
    } else {
        class.to_string()
    };
    // `col-resize` / `row-resize` are stripped from `main.css` by the Dioxus
    // asset pipeline; set them inline so hover still shows the right cursor.
    let cursor = match axis {
        SplitAxis::Folder | SplitAxis::ListWidth => "cursor: col-resize;",
        SplitAxis::List => "cursor: row-resize;",
    };
    let mut handle_el = use_signal(|| None::<web_sys::Element>);

    rsx! {
        div {
            class: class,
            style: cursor,
            role: "separator",
            aria_orientation: orientation,
            tabindex: "0",
            aria_label: "Drag to resize. Double-click to reset.",
            onmounted: move |evt| {
                if let Some(el) = evt.data().downcast::<web_sys::Element>() {
                    let _ = el.set_attribute("style", cursor);
                    handle_el.set(Some(el.clone()));
                }
            },
            onpointerdown: move |evt| {
                evt.prevent_default();
                let id = evt.data().pointer_id();
                if let Some(el) = handle_el.peek().as_ref() {
                    let _ = el.set_pointer_capture(id);
                }
                dragging.set(true);
            },
            onpointermove: move |evt| {
                if !*dragging.peek() {
                    return;
                }
                let pt = evt.data().client_coordinates();
                match axis {
                    SplitAxis::Folder => {
                        set_folder_width_px(clamp_folder_width_px(pt.x));
                    }
                    SplitAxis::List => apply_list_drag(pt.y),
                    SplitAxis::ListWidth => apply_list_width_drag(pt.x),
                }
            },
            onpointerup: move |_| {
                if !*dragging.peek() {
                    return;
                }
                persist_layout();
                dragging.set(false);
            },
            onpointercancel: move |_| {
                if !*dragging.peek() {
                    return;
                }
                persist_layout();
                dragging.set(false);
            },
            ondoubleclick: move |_| {
                match axis {
                    SplitAxis::Folder => reset_folder_width(),
                    SplitAxis::List => reset_list_height(),
                    SplitAxis::ListWidth => reset_list_width(),
                }
            },
        }
    }
}

fn apply_list_width_drag(client_x: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(list) = document.get_element_by_id("messagelist") else {
            return;
        };
        let list_left = list.get_bounding_client_rect().x();
        let px = clamp_list_width_px(client_x - list_left);
        set_list_width_px(px);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = client_x;
    }
}

fn apply_list_drag(client_y: f64) {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Some(list) = document.get_element_by_id("messagelist") else {
            return;
        };
        let Some(view) = document.get_element_by_id("messageview") else {
            return;
        };
        let list_top = list.get_bounding_client_rect().y();
        let view_bottom = view.get_bounding_client_rect().bottom();
        let total = view_bottom - list_top;
        if total <= 1.0 {
            return;
        }
        let pct = clamp_list_height_pct((client_y - list_top) / total * 100.0);
        set_list_height_pct(pct);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = client_y;
    }
}
