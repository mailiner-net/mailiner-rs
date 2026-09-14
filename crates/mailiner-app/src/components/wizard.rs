//! Reusable multi-step wizard chrome (progress, back / primary, step body).

use dioxus::prelude::*;

use crate::a11y::focus_first_focusable;

/// Presentational wizard frame. Not account-aware.
#[component]
pub fn WizardShell(
    title: String,
    subtitle: String,
    step_index: usize,
    step_count: usize,
    primary_label: String,
    show_back: bool,
    busy: bool,
    on_back: EventHandler<()>,
    on_primary: EventHandler<()>,
    #[props(default)] eyebrow: String,
    children: Element,
) -> Element {
    let step_n = step_index.saturating_add(1);
    let progress_label = crate::i18n::t_args(
        "wizard.step_of",
        &[
            ("n", &step_n.to_string()),
            ("m", &step_count.max(1).to_string()),
        ],
    );

    let mut focused_step = use_signal(|| None::<usize>);
    if focused_step() != Some(step_index) {
        focused_step.set(Some(step_index));
        spawn(async move {
            #[cfg(target_arch = "wasm32")]
            {
                gloo_timers::future::TimeoutFuture::new(50).await;
            }
            focus_first_focusable(".wizard-body");
        });
    }

    rsx! {
        form {
            class: "onboarding-form wizard-form",
            onsubmit: move |evt| {
                evt.prevent_default();
                if !busy {
                    on_primary.call(());
                }
            },
            if !eyebrow.is_empty() {
                p { class: "wizard-eyebrow", "{eyebrow}" }
            }
            if step_count > 1 {
                div {
                    class: "wizard-progress",
                    role: "group",
                    aria_label: crate::i18n::t("wizard.progress"),
                    p { class: "wizard-step-count bootstrap-muted", "{progress_label}" }
                    div {
                        class: "wizard-dots",
                        for i in 0..step_count {
                            span {
                                key: "{i}",
                                class: if i == step_index {
                                    "wizard-dot is-current"
                                } else if i < step_index {
                                    "wizard-dot is-done"
                                } else {
                                    "wizard-dot"
                                },
                                aria_current: if i == step_index { "step" } else { "false" },
                            }
                        }
                    }
                }
            }
            h1 { class: "bootstrap-title", "{title}" }
            if !subtitle.is_empty() {
                p { class: "bootstrap-muted wizard-subtitle", "{subtitle}" }
            }
            div { class: "wizard-body", {children} }
            div {
                class: "onboarding-actions wizard-actions",
                if show_back {
                    button {
                        r#type: "button",
                        class: "onboarding-btn onboarding-btn-secondary",
                        disabled: busy,
                        onclick: move |_| on_back.call(()),
                        {crate::i18n::t("wizard.back")}
                    }
                }
                button {
                    r#type: "submit",
                    class: "onboarding-btn onboarding-btn-primary",
                    disabled: busy,
                    "{primary_label}"
                }
            }
        }
    }
}
