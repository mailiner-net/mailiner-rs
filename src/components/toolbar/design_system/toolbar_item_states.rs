use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;
use dioxus_free_icons::Icon;
use dioxus_free_icons::icons::ld_icons::{LdMail, LdTrash2};
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
                        Icon {
                            icon: LdMail,
                            class: class!(w_5 h_5),
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
                        Icon {
                            icon: LdMail,
                            class: class!(w_5 h_5),
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
                        Icon {
                            icon: LdMail,
                            class: class!(w_5 h_5),
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
                        Icon {
                            icon: LdTrash2,
                            class: class!(w_5 h_5),
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