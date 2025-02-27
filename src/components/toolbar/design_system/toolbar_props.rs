use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

/// Component to display toolbar props documentation
pub fn ToolbarProps() -> Element {
    rsx! {
        div { 
            class: class!(bg_white p_4 rounded border border_neutral_200 shadow_sm col_span_1 md(col_span_2)),
            h3 { 
                class: class!(text_lg font_medium mb_3), 
                "Toolbar Props" 
            }
            
            div { 
                class: class!(overflow_x_auto),
                table { 
                    class: class!(min_w_full divide_y divide_neutral_200),
                    thead { 
                        class: class!(bg_neutral_50),
                        tr {
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Prop" 
                            }
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Type" 
                            }
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Default" 
                            }
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Description" 
                            }
                        }
                    }
                    tbody { 
                        class: class!(divide_y divide_neutral_200 bg_white),
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "items" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Vec<ToolbarItemData>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Required" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Array of toolbar item data to display" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "position" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<ToolbarPosition>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Top" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Toolbar position (Top, Bottom, Left, Right)" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "size" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<ToolbarSize>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Medium" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Toolbar size (Small, Medium, Large)" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "show_labels" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<bool>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "true" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Whether to show text labels alongside icons" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "show_dividers" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<bool>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "true" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Whether to show dividers between items" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "class" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<String>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "None" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Additional CSS classes for the toolbar" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "on_item_click" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<EventHandler<String>>" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "None" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Event handler when a toolbar item is clicked" 
                            }
                        }
                    }
                }
            }
            
            // ToolbarItemData Properties
            h4 { 
                class: class!(text_lg font_medium mt_6 mb_3), 
                "ToolbarItemData Properties" 
            }
            
            div { 
                class: class!(overflow_x_auto),
                table { 
                    class: class!(min_w_full divide_y divide_neutral_200),
                    thead { 
                        class: class!(bg_neutral_50),
                        tr {
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Property" 
                            }
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Type" 
                            }
                            th { 
                                class: class!(px_3 py_2 text_left text_sm font_medium text_neutral_700), 
                                "Description" 
                            }
                        }
                    }
                    tbody { 
                        class: class!(divide_y divide_neutral_200 bg_white),
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "id" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "String" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Unique identifier for the toolbar item" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "icon" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Element" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Icon element to display (usually an SVG)" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "label" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<String>" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Optional text label for the item" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "tooltip" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<String>" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Optional tooltip text on hover" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "disabled" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<bool>" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Whether the item is in a disabled state" 
                            }
                        }
                        tr {
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_800 font_medium), 
                                "danger" 
                            }
                            td { 
                                class: class!(px_3 py_2 whitespace_nowrap text_sm text_neutral_600), 
                                "Option<bool>" 
                            }
                            td { 
                                class: class!(px_3 py_2 text_sm text_neutral_600), 
                                "Whether the item represents a destructive action" 
                            }
                        }
                    }
                }
            }
        }
    }
}
