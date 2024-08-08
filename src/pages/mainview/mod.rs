use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

mod header;
mod accountsview;
mod messagelist;
mod messageview;
mod footer;

use header::Header;
use accountsview::AccountsView;
use messagelist::MessageList;
use messageview::MessageView;
use footer::Footer;


#[component]
pub fn MainView() -> Element {
    rsx! {
        div {
            class: class!(h_full w_full flex flex_col),

            div {
                class: class!(bg_gray_800),
                Header {}
            },

            div {
                class: class!(flex grow),

                div {
                    class: class!(w_64 bg_gray_900),
                    AccountsView {}
                },

                div {
                    class: class!(flex flex_col grow bg_gray_800),

                    div {
                        class: class!(h_16 bg_gray_700 grow),
                        MessageList {}
                    },

                    div {
                        class: class!(min_h_2_half bg_gray_600),
                        MessageView {}
                    }
                }
            },

            div {
                class: class!(bg_gray_800),
                Footer {}
            }
        }
    }
}