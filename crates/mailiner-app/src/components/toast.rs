//! Toast host: timeout, hover-pause, and the central Undo control.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::*;

use crate::context::AppContext;
use crate::core_event::CoreEvent;
use crate::toast::{TOAST_TICK_MS, Toast};

#[component]
pub fn ToastHost() -> Element {
    let mut ctx = use_context::<AppContext>();
    let core = use_coroutine_handle::<CoreEvent>();
    let toast = ctx.toast.read().clone();
    let timeout_ms = toast.as_ref().map(|t| t.action.timeout_ms()).unwrap_or(0);
    let mut remaining_ms = use_signal(|| timeout_ms);
    let mut hovered = use_signal(|| false);
    let mut suppress_dismiss = use_signal(|| None::<u64>);
    let prev = use_hook(|| Rc::new(RefCell::new(None::<Toast>)));

    {
        let prev = prev.clone();
        use_effect(move || {
            let current = ctx.toast.read().clone();
            let previous = prev.borrow_mut().take();
            if let Some(old) = previous {
                if current.as_ref().is_some_and(|t| t.id == old.id) {
                    *prev.borrow_mut() = Some(old);
                    return;
                }
                if *suppress_dismiss.peek() != Some(old.id) {
                    if let Some(commit) = old.action.on_dismiss() {
                        let _ = core.send(CoreEvent::CommitDismissed(commit));
                    }
                }
            }
            *prev.borrow_mut() = current.clone();

            let Some(t) = current else {
                return;
            };
            remaining_ms.set(t.action.timeout_ms());
            let id = t.id;
            let total = t.action.timeout_ms();
            spawn(async move {
                let mut left = total;
                while left > 0 {
                    sleep_ms(TOAST_TICK_MS).await;
                    if ctx.toast.peek().as_ref().map(|x| x.id) != Some(id) {
                        return;
                    }
                    if *hovered.peek() {
                        continue;
                    }
                    left = left.saturating_sub(TOAST_TICK_MS);
                    remaining_ms.set(left);
                }
                if ctx.toast.peek().as_ref().map(|x| x.id) == Some(id) {
                    ctx.toast.set(None);
                }
            });
        });
    }

    let Some(toast) = toast else {
        return rsx! {};
    };
    let progress = if timeout_ms == 0 {
        0.0
    } else {
        (*remaining_ms.read() as f64 / timeout_ms as f64).clamp(0.0, 1.0)
    };
    let undo_label = toast.action.undo_label();
    let message = toast.action.message();

    rsx! {
        div {
            class: "toast",
            role: "status",
            aria_live: "polite",
            aria_atomic: "true",
            onmouseenter: move |_| hovered.set(true),
            onmouseleave: move |_| hovered.set(false),
            div {
                class: "toast-body",
                span { class: "toast-message", "{message}" }
                if let Some(label) = undo_label {
                    button {
                        class: "toast-undo",
                        r#type: "button",
                        onclick: {
                            let toast = toast.clone();
                            move |_| {
                                suppress_dismiss.set(Some(toast.id));
                                let undo = toast.action.undo();
                                ctx.toast.set(None);
                                if let Some(undo) = undo {
                                    let _ = core.send(CoreEvent::Undo(undo));
                                }
                            }
                        },
                        "{label}"
                    }
                }
                button {
                    class: "toast-close",
                    r#type: "button",
                    aria_label: "Dismiss",
                    onclick: move |_| ctx.toast.set(None),
                    "×"
                }
            }
            div {
                class: "toast-timer",
                aria_hidden: "true",
                style: "--toast-progress: {progress}",
            }
        }
    }
}

async fn sleep_ms(ms: u32) {
    #[cfg(target_arch = "wasm32")]
    {
        gloo_timers::future::TimeoutFuture::new(ms).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::time::sleep(std::time::Duration::from_millis(ms as u64)).await;
    }
}
