use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use mailiner_css::*;

/// Component to display different toolbar item states
pub fn ToolbarItemStates() -> Element {
    rsx! {
        div { 
            class: class!(bg_white p_4 rounded border border_neutral_200 shadow_sm),
            h3 { 
                class: class!(text_lg font_medium mb_3), 
                "Toolbar Item States" 
            }
            
            div { 
                class: class!(space_y_3),
                div { 
                    class: class!(flex items_center p_3 border border_neutral_200 rounded),
                    div { 
                        class: class!(w_10 h_10 bg_neutral_50 flex items_center justify_center rounded text_neutral_700),
                        svg {
                            class: class!(w_5 h_5),
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                            }
                        }
                    }
                    div { 
                        class: class!(ml_3),
                        h4 { class: class!(font_medium), "Normal State" }
                        p { class: class!(text_sm text_neutral_600), "Default toolbar item appearance" }
                    }
                }
                
                div { 
                    class: class!(flex items_center p_3 border border_neutral_200 rounded),
                    div { 
                        class: class!(w_10 h_10 bg_neutral_100 flex items_center justify_center rounded text_neutral_700),
                        svg {
                            class: class!(w_5 h_5),
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                            }
                        }
                    }
                    div { 
                        class: class!(ml_3),
                        h4 { class: class!(font_medium), "Hover State" }
                        p { class: class!(text_sm text_neutral_600), "When mouse is hovering over item" }
                    }
                }
                
                div { 
                    class: class!(flex items_center p_3 border border_neutral_200 rounded),
                    div { 
                        class: class!(w_10 h_10 bg_neutral_50 flex items_center justify_center rounded text_neutral_400 cursor_not_allowed),
                        svg {
                            class: class!(w_5 h_5),
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
                            }
                        }
                    }
                    div { 
                        class: class!(ml_3),
                        h4 { class: class!(font_medium), "Disabled State" }
                        p { class: class!(text_sm text_neutral_600), "When item is not interactive" }
                    }
                }
                
                div { 
                    class: class!(flex items_center p_3 border border_neutral_200 rounded),
                    div { 
                        class: class!(w_10 h_10 bg_danger_50 flex items_center justify_center rounded text_danger_600),
                        svg {
                            class: class!(w_5 h_5),
                            xmlns: "http://www.w3.org/2000/svg",
                            view_box: "0 0 24 24",
                            fill: "none",
                            stroke: "currentColor",
                            
                            path {
                                stroke_linecap: "round",
                                stroke_linejoin: "round",
                                stroke_width: "2",
                                d: "M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                            }
                        }
                    }
                    div { 
                        class: class!(ml_3),
                        h4 { class: class!(font_medium), "Danger State" }
                        p { class: class!(text_sm text_neutral_600), "For destructive actions" }
                    }
                }
            }
        }
    }
}