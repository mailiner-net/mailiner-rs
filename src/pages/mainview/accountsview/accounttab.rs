use dioxus::prelude::*;
use dioxus_daisyui::prelude::*;
use uuid::Uuid;
use mailiner_core::settings::use_accounts;


#[component]
pub fn AccountTab(account_id: Uuid) -> Element {
    let accounts = use_accounts();
    let account = use_memo(move || accounts.read().get(&account_id).unwrap().clone() );

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
