//! The daemon's long-term keys (§2.2): X25519 to receive sealed frames, Ed25519 to sign them.
//! ponytail: a 0600 file in the daemon home, not the Keychain the PRD names. The Keychain choice
//! is review item 2 (which accessibility class a launch agent can rely on while the Mac is locked),
//! and an ad hoc signed dev build would hit a Keychain prompt on every rebuild.

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};
use swarn_core::crypto::{self, SigningKey, ed25519_dalek};

pub struct Identity {
    pub x25519_secret: [u8; 32],
    pub x25519_public: [u8; 32],
    pub signing: SigningKey,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    x25519: String,
    ed25519: String,
}

impl Identity {
    pub fn load_or_create(home: &Path) -> std::io::Result<Self> {
        let path = home.join("identity.json");
        let stored = match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Stored>(&text).map_err(std::io::Error::other)?,
            Err(_) => {
                let (secret, _) = crypto::x25519_generate();
                let stored = Stored {
                    x25519: B64.encode(secret),
                    ed25519: B64.encode(crypto::ed25519_generate().to_bytes()),
                };
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)?;
                std::io::Write::write_all(&mut file, serde_json::to_string(&stored)?.as_bytes())?;
                stored
            }
        };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let bytes = |s: &str| -> std::io::Result<[u8; 32]> {
            B64.decode(s)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| std::io::Error::other("corrupt identity.json"))
        };
        let x25519_secret = bytes(&stored.x25519)?;
        Ok(Self {
            x25519_public: crypto::x25519_public(&x25519_secret).map_err(std::io::Error::other)?,
            x25519_secret,
            signing: SigningKey::Ed25519(ed25519_dalek::SigningKey::from_bytes(&bytes(
                &stored.ed25519,
            )?)),
        })
    }
}
