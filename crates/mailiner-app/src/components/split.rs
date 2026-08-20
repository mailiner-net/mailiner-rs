//! Drag handles between mail chrome panes.

use dioxus::prelude::*;

use crate::layout::{
    clamp_folder_width_px, clamp_list_height_pct, persist_layout, reset_folder_width,
    reset_list_height, set_folder_width_px, set_list_height_pct,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Folder tree vs list/viewer (vertical bar, col-resize).
    Folder,
    /// Message list vs viewer (horizontal bar, row-resize).
    List,
}

#[component]
pub fn SplitHandle(axis: SplitAxis) -> Element {
    let mut dragging = use_signal(|| false);
    let class = match axis {
        SplitAxis::Folder => "split-handle split-handle-col",
        SplitAxis::List => "split-handle split-handle-row",
    };
    let orientation = match axis {
        SplitAxis::Folder => "vertical",
        SplitAxis::List => "horizontal",
    };

    rsx! {
        div {
            class: class,
            role: "separator",
            aria_orientation: orientation,
            tabindex: "0",
            title: "Drag to resize. Double-click to reset.",
            onpointerdown: move |evt| {
                evt.prevent_default();
                dragging.set(true);
            },
            ondoubleclick: move |_| {
                match axis {
                    SplitAxis::Folder => reset_folder_width(),
                    SplitAxis::List => reset_list_height(),
                }
            },
        }
        if dragging() {
            div {
                class: match axis {
                    SplitAxis::Folder => "split-drag-overlay split-drag-overlay-col",
                    SplitAxis::List => "split-drag-overlay split-drag-overlay-row",
                },
                onpointermove: move |evt| {
                    let pt = evt.data().client_coordinates();
                    match axis {
                        SplitAxis::Folder => {
                            set_folder_width_px(clamp_folder_width_px(pt.x));
                        }
                        SplitAxis::List => apply_list_drag(pt.y),
                    }
                },
                onpointerup: move |_| {
                    persist_layout();
                    dragging.set(false);
                },
                onpointercancel: move |_| {
                    persist_layout();
                    dragging.set(false);
                },
            }
        }
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
