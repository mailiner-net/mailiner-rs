//! Mailiner envelope mark, shared with the marketing site.

use dioxus::prelude::*;

/// Envelope body (`#a2deeb`). Keep in sync with `assets/mailiner-logo.svg`.
const LOGO_BODY: &str = "M0 23.311a4.072 4.072 0 0 1 4.072-4.072c1.187 0 1.864.451 2.996 1.32 11.576 8.91 48.078 38.978 48.078 38.978 1.477 1.187 3.892 1.187 5.369 0 0 0 36.55-29.702 48.264-39.168.787-.635 1.72-1.13 2.812-1.13 2.206 0 4 1.76 4.064 3.951l.006 4.904v44.77a3.453 3.453 0 0 1-3.443 3.445H3.443A3.453 3.453 0 0 1 0 72.864V25.96Z";
/// Envelope flap (`#ee4266`).
const LOGO_FLAP: &str = "M57.83 50.782c-1.765-.023-2.857-.884-3.63-1.489C41.807 39.625 4.51 8.141 4.51 8.141l.01-.032a4.567 4.567 0 0 1-1.672-3.536c0-2.358 1.787-4.3 4.083-4.546L6.94 0h101.783l.008.027a4.576 4.576 0 0 1 4.083 4.546 4.568 4.568 0 0 1-1.672 3.536l.01.032S76.197 36.65 61.404 49.293c-.673.575-1.947 1.511-3.574 1.489z";

fn logo_width(height: u32) -> u32 {
    let width = (u64::from(height) * 115661) / 76309;
    u32::try_from(width).unwrap_or(height).max(1)
}

/// Two-tone envelope. `height` is in CSS pixels; width follows the viewBox.
#[component]
pub fn MailinerLogo(#[props(default = 32)] height: u32) -> Element {
    let width = logo_width(height);
    rsx! {
        svg {
            class: "mailiner-logo",
            xmlns: "http://www.w3.org/2000/svg",
            view_box: "0 0 115.661 76.309",
            width: "{width}",
            height: "{height}",
            "aria-hidden": "true",
            path { d: LOGO_BODY, fill: "#a2deeb" }
            path { d: LOGO_FLAP, fill: "#ee4266" }
        }
    }
}

/// Envelope plus the serif wordmark, for full-page shells.
#[component]
pub fn BrandLockup() -> Element {
    rsx! {
        div {
            class: "brand-lockup",
            MailinerLogo { height: 32 }
            span { class: "brand-wordmark", "Mailiner" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_paths_match_asset_and_splash() {
        let svg = include_str!("../../assets/mailiner-logo.svg");
        let html = include_str!("../../index.html");
        assert!(svg.contains(LOGO_BODY), "logo svg body drifted");
        assert!(svg.contains(LOGO_FLAP), "logo svg flap drifted");
        assert!(html.contains(LOGO_BODY), "splash logo body drifted");
        assert!(html.contains(LOGO_FLAP), "splash logo flap drifted");
        assert!(svg.contains("#a2deeb") && svg.contains("#ee4266"));
    }
}
