use dioxus::prelude::*;
use dioxus_tailwindcss::prelude::*;

mod button;
mod checkbox;
mod dialog;
mod input;
mod radio;
mod sidebar;
mod toolbar;

pub use button::{Button, ButtonProps, ButtonSize, ButtonVariant};
pub use checkbox::{Checkbox, CheckboxProps};
pub use dialog::{
    ConfirmDialog, ConfirmDialogProps, Dialog, DialogProps, DialogSize, DialogVariant,
};
pub use input::{Input, InputProps, InputSize, InputType};
pub use radio::{RadioButton, RadioButtonProps, RadioGroup, RadioGroupDirection, RadioGroupProps};
pub use sidebar::{Sidebar, SidebarItemData};
pub use toolbar::{
    ButtonGroupToolbar, ButtonGroupToolbarProps, Toolbar, ToolbarItemData, ToolbarPosition,
    ToolbarProps, ToolbarSize,
};

use button::Gallery as ButtonGallery;
use checkbox::Gallery as CheckboxGallery;
use dialog::Gallery as DialogGallery;
use input::Gallery as InputGallery;
use radio::Gallery as RadioGallery;
use sidebar::Gallery as SidebarGallery;
use toolbar::ToolbarDesignSystem;

#[derive(Clone, Debug, PartialEq, Routable)]
enum GalleryRoute {
    #[layout(ComponentGalleryLayout)]
    #[route("/")]
    ComponentGalleryOverview {},

    #[route("/button")]
    ButtonGallery {},

    #[route("/checkbox")]
    CheckboxGallery {},

    #[route("/radio")]
    RadioGallery {},

    #[route("/input")]
    InputGallery {},

    #[route("/dialog")]
    DialogGallery {},

    #[route("/sidebar")]
    SidebarGallery {},

    #[route("/toolbar")]
    ToolbarDesignSystem {},
}

#[component]
pub fn ComponentGalleryOverview() -> Element {
    rsx! {
        "Component Gallery Overview"
    }
}

#[component]
pub fn ComponentGalleryLayout() -> Element {
    rsx! {
        div {
            class: class!(flex flex_row gap_5),

            div {
                class: class!(flex flex_col gap_5),

                div {
                    Link {
                        to: GalleryRoute::ButtonGallery {},
                        "Button"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::CheckboxGallery {},
                        "Checkbox"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::RadioGallery {},
                        "Radio"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::InputGallery {},
                        "Input"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::DialogGallery {},
                        "Dialog"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::SidebarGallery {},
                        "Sidebar"
                    }
                }

                div {
                    Link {
                        to: GalleryRoute::ToolbarDesignSystem {},
                        "Toolbar"
                    }
                }
            }
            div {
                class: class!(grow),

                Outlet::<GalleryRoute> {}
            }
        }
    }
}

#[component]
pub fn ComponentGallery() -> Element {
    rsx! {
        Router::<GalleryRoute> {}
    }
}
