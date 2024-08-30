use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use mailiner_core::imap_account_manager::use_imap_account_manager;

mod accounttab;

use accounttab::AccountTab;
use uuid::Uuid;

#[derive(Clone, PartialEq, Props)]
pub struct Props {
    class: String,
}

#[component]
pub fn AccountsView() -> Element {
    let account_manager = use_imap_account_manager();
    let ids = use_memo(move || {
        let manager_ref = account_manager.read();
        manager_ref
            .accounts()
            .into_iter()
            .map(|account| account.read().id.clone())
            .collect::<Vec<Uuid>>()
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
