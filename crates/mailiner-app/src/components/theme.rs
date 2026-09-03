//! Color theme control (System / Light / Dark).

use dioxus::prelude::*;

use crate::ui_prefs::{self, ThemePref};

/// Persist and apply a theme change. Shared by settings and the mailbox header.
pub fn set_theme(mut theme: Signal<ThemePref>, next: ThemePref) {
    theme.set(next);
    ui_prefs::save_theme(next);
    ui_prefs::apply_theme(next);
}

#[component]
pub fn ThemeSelect(
    #[props(default)] id: String,
    #[props(default = "theme-select".to_string())] class: String,
) -> Element {
    let theme = use_context::<Signal<ThemePref>>();
    let current = theme();
    rsx! {
        select {
            id: id,
            class: "{class}",
            aria_label: "Color theme",
            title: "Color theme",
            value: "{current.as_key()}",
            onchange: move |evt| {
                if let Some(next) = ThemePref::from_key(&evt.value()) {
                    set_theme(theme, next);
                }
            },
            for option in ThemePref::ALL {
                option {
                    value: "{option.as_key()}",
                    selected: option == current,
                    "{option.label()}"
                }
            }
        }
    }
}
