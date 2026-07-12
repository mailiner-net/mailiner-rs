//! Composer "From" identity (app maps Account → this type).

/// App-supplied sender identity. Plain data — no trait object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FromIdentity {
    /// Display name shown in From.
    pub display_name: String,
    /// Mailbox address (required).
    pub email: String,
}

impl FromIdentity {
    /// Construct a sender identity.
    pub fn new(display_name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            email: email.into(),
        }
    }
}
