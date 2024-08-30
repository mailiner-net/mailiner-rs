use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use mailiner_core::imap_account_manager::use_imap_account_manager;
use uuid::Uuid;

#[component]
pub fn AccountTab(account_id: Uuid) -> Element {
    let manager = use_imap_account_manager();
    let account = manager.read().get_account(account_id).unwrap();

    rsx! {
        div {
            class: class!(collapse collapse_plus border_base_300 bg_base_200 border),
            input {
                r#type: checkbox,
                checked: true
            }

            div {
                class: class!(collapse_title text_xl font_medium),
                { account().name.clone() }
            },

            div {
                class: class!(collapse_content),
                { "Foo "}
            }
        }
    }
}
