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

            Header {},

            div {
                class: class!(flex grow),

                aside {
                    class: class!(w_80 bg_gray_100),

                    AccountsView {}
                }

                div {
                    class: class!(flex flex_col grow bg_gray_800),

                    main {
                        class: class!(h_16 bg_gray_700 grow),
                        MessageList {}
                    },

                    article  {
                        class: class!(min_h_2_half bg_gray_600),
                        MessageView {}
                    }
                }
            },

            footer {
                class: class!(bg_gray_800),
                Footer {}
            }
        }
    }
}