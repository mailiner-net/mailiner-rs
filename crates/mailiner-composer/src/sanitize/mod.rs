//! Edit/export HTML sanitize policy wrappers over [`mailiner_html`].

pub use mailiner_html::{
    is_safe_data_image, sanitize_css, sanitize_for_edit, sanitize_for_export, SAFE_IMAGE_TYPES,
};
