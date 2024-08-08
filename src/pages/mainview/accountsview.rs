use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use crate::corelib::settings::use_settings;

#[component]
pub fn AccountsView() -> Element {
    let settings = use_settings().get();
    let rendered_accounts = settings.accounts.iter().map(|account| {
        rsx! {
            div {
                id: account.id.clone(),
                class: class!(flex flex_col gap_2),
                h2 {
                    class: class!(text_lg),
                    { account.name.clone() }
                }
                p {
                    class: class!(text_sm),
                    { account.email.clone() }
                }
            }
        }
    });

    rsx! {
        div {
            class: class!(h_full w_full flex flex_col),

            { rendered_accounts }
        }
    }
}