//! Global shortcut listener and the **?** help dialog.

use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::icons::{IconButton, IconKind};
use super::messageview::{MessageScroll, scroll_message_view};
use crate::context::{AppContext, MailboxPickerMode};
use crate::core_event::CoreEvent;
use crate::shortcuts::{
    GLOBAL_SHORTCUTS, ShortcutGroup, ShortcutId, shortcut_for_key, shortcuts_in_group,
};
use crate::toast::ToastAction;

fn claim_shortcut(evt: &web_sys::KeyboardEvent) {
    evt.prevent_default();
    evt.stop_propagation();
}

fn event_target_is_editable(evt: &web_sys::KeyboardEvent) -> bool {
    let Some(target) = evt.target() else {
        return false;
    };
    let Ok(el) = target.dyn_into::<web_sys::HtmlElement>() else {
        return false;
    };
    if el.is_content_editable() {
        return true;
    }
    matches!(el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
}

struct WindowKeydown {
    closure: Closure<dyn FnMut(web_sys::KeyboardEvent)>,
}

impl Drop for WindowKeydown {
    fn drop(&mut self) {
        if let Some(win) = web_sys::window() {
            let _ = win.remove_event_listener_with_callback_and_bool(
                "keydown",
                self.closure.as_ref().unchecked_ref(),
                true,
            );
        }
    }
}

fn run_shortcut(id: ShortcutId, ctx: &mut AppContext, core: Coroutine<CoreEvent>, help_open: &mut Signal<bool>) {
    match id {
        ShortcutId::JumpToFolder => {
            ctx.mailbox_picker.set(Some(MailboxPickerMode::Jump));
        }
        ShortcutId::MoveToFolder => {
            if ctx.selected_message.peek().is_none() {
                ctx.show_toast(ToastAction::info("Select a message first"));
                return;
            }
            ctx.mailbox_picker.set(Some(MailboxPickerMode::Move));
        }
        ShortcutId::NextMessage => {
            let _ = core.send(CoreEvent::SelectAdjacent { delta: 1 });
        }
        ShortcutId::PrevMessage => {
            let _ = core.send(CoreEvent::SelectAdjacent { delta: -1 });
        }
        ShortcutId::ScrollMessageDown => {
            scroll_message_view(true, MessageScroll::Line);
        }
        ShortcutId::ScrollMessageUp => {
            scroll_message_view(false, MessageScroll::Line);
        }
        ShortcutId::PageMessageDown => {
            scroll_message_view(true, MessageScroll::Page);
        }
        ShortcutId::PageMessageUp => {
            scroll_message_view(false, MessageScroll::Page);
        }
        ShortcutId::ShowHelp => {
            help_open.set(true);
        }
    }
}

#[component]
pub fn ShortcutsHost() -> Element {
    let ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let mut help_open = use_signal(|| false);
    let help = *help_open.read();

    use_hook(|| {
        let mut ctx = ctx.clone();
        let mut help_open = help_open;
        let core = core;
        let closure = Closure::wrap(Box::new(move |evt: web_sys::KeyboardEvent| {
            if evt.ctrl_key() || evt.meta_key() || evt.alt_key() {
                return;
            }
            if event_target_is_editable(&evt) {
                return;
            }
            if ctx.compose_draft.peek().is_some() {
                return;
            }

            if *help_open.peek() {
                let key = evt.key();
                if key == "Escape" || shortcut_for_key(&key).is_some_and(|s| s.id == ShortcutId::ShowHelp)
                {
                    claim_shortcut(&evt);
                    help_open.set(false);
                } else if shortcut_for_key(&key).is_some() {
                    claim_shortcut(&evt);
                }
                return;
            }

            if ctx.mailbox_picker.peek().is_some() {
                return;
            }

            let Some(shortcut) = shortcut_for_key(&evt.key()) else {
                return;
            };
            claim_shortcut(&evt);
            run_shortcut(shortcut.id, &mut ctx, core, &mut help_open);
        }) as Box<dyn FnMut(_)>);
        if let Some(win) = web_sys::window() {
            let _ = win.add_event_listener_with_callback_and_bool(
                "keydown",
                closure.as_ref().unchecked_ref(),
                true,
            );
        }
        Rc::new(WindowKeydown { closure })
    });

    if !help {
        return rsx! {};
    }

    rsx! {
        ShortcutHelp {
            onclose: move |_| help_open.set(false),
        }
    }
}

#[component]
fn ShortcutHelp(onclose: EventHandler<MouseEvent>) -> Element {
    rsx! {
        div {
            class: "picker-backdrop shortcut-backdrop",
            onclick: move |evt| onclose.call(evt),
            div {
                class: "ui-dialog shortcut-dialog",
                role: "dialog",
                aria_modal: "true",
                aria_label: "Keyboard shortcuts",
                onclick: move |evt| evt.stop_propagation(),
                div {
                    class: "ui-dialog-head",
                    h2 { class: "ui-dialog-title", "Keyboard shortcuts" }
                    IconButton {
                        class: "flat ui-icon-btn",
                        title: "Close",
                        size: 20,
                        icon: IconKind::XMark,
                        onclick: move |evt| onclose.call(evt),
                    }
                }
                for group in ShortcutGroup::ALL {
                    section {
                        class: "shortcut-group",
                        h3 { class: "shortcut-group-title", "{group.title()}" }
                        ul {
                            class: "shortcut-list",
                            for shortcut in shortcuts_in_group(*group) {
                                li {
                                    class: "shortcut-row",
                                    span { class: "shortcut-desc", "{shortcut.description}" }
                                    kbd { class: "shortcut-key", "{shortcut.label}" }
                                }
                            }
                        }
                    }
                }
                p {
                    class: "picker-hint",
                    "Esc to close"
                }
            }
        }
    }
}
