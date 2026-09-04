//! Readable card for `text/calendar` invites (title, time, organizer).

use dioxus::prelude::*;
use mailiner_mime::CalendarInvite;

#[component]
pub fn CalendarInviteCards(invites: Vec<CalendarInvite>) -> Element {
    if invites.is_empty() {
        return rsx! {};
    }
    rsx! {
        div {
            class: "calendar-invites",
            for (i, invite) in invites.into_iter().enumerate() {
                CalendarInviteCard { key: "{i}", invite }
            }
        }
    }
}

#[component]
fn CalendarInviteCard(invite: CalendarInvite) -> Element {
    let kind = invite.kind_label();
    let title = invite.title();
    let when = invite.time_label();
    let organizer = invite
        .organizer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let location = invite
        .location
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let cancelled = kind == "Cancelled";
    rsx! {
        article {
            class: if cancelled {
                "calendar-invite is-cancelled"
            } else {
                "calendar-invite"
            },
            header {
                class: "calendar-invite-kicker",
                "{kind}"
            }
            h3 { class: "calendar-invite-title", "{title}" }
            if let Some(when) = when {
                div {
                    class: "calendar-invite-row",
                    span { class: "calendar-invite-label", "When" }
                    span { class: "calendar-invite-value", "{when}" }
                }
            }
            if let Some(organizer) = organizer {
                div {
                    class: "calendar-invite-row",
                    span { class: "calendar-invite-label", "Organizer" }
                    span { class: "calendar-invite-value", "{organizer}" }
                }
            }
            if let Some(location) = location {
                div {
                    class: "calendar-invite-row",
                    span { class: "calendar-invite-label", "Where" }
                    span { class: "calendar-invite-value", "{location}" }
                }
            }
        }
    }
}
