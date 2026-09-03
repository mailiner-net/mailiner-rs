//! Edit/export HTML sanitize policy wrappers over [`mailiner_html`].

pub use mailiner_html::{
    is_safe_data_image, is_safe_image_content_type, sanitize_css, sanitize_for_edit,
    sanitize_for_export, SAFE_IMAGE_ACCEPT, SAFE_IMAGE_TYPES,
};
