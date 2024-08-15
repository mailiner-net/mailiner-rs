use mailiner_core::settings::use_accounts;
use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;

mod accounttab;

use accounttab::AccountTab;

#[derive(Clone, PartialEq, Props)]
pub struct Props {
    class: String,
}

#[component]
pub fn AccountsView() -> Element {
    let accounts = use_accounts();
    let ids = use_memo(move || {
        accounts
            .read()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    });

    rsx! {
        div {
            class: class!(h_full w_full flex flex_col),

            for id in ids() {
                AccountTab {
                    key: "{id.to_string()}",
                    account_id: id
                }
            }
        }
    }
}
