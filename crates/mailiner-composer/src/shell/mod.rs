//! Full email composer shell (recipients, subject, attachments, toolbar).

pub mod attachment_list;
pub mod email_composer;
pub mod recipient_field;

pub use email_composer::{
    apply_preferred_mode, capture_live_body, editor_mount_html, prepare_export_bodies,
    switch_body_mode, SwitchBodyResult,
};
