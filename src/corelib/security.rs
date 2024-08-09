use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum Security {
    None,
    Ssl,
    StartTls
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "authentication")]
pub enum Authentication {
    Plain {
        username: String,
        password: String
    },
    Login {
        username: String,
        password: String
    },
    XOAuth2 {
        token: String
    }
}