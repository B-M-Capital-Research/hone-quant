//! Authenticated encryption for credentials stored in the database (notification channel
//! tokens, webhook URLs, SMTP passwords). The key comes from `HONE_QUANT_SECRET_KEY` or the
//! auto-generated `secret.key` in the state directory, so a database dump alone never reveals
//! the secrets.

use anyhow::{Result, anyhow};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::RngCore;

const NONCE_LEN: usize = 12;

pub struct SecretBox {
    cipher: ChaCha20Poly1305,
}

impl SecretBox {
    pub fn new(key: &[u8; 32]) -> Self {
        Self {
            cipher: ChaCha20Poly1305::new(Key::from_slice(key)),
        }
    }

    /// `base64(nonce || ciphertext)`.
    pub fn seal(&self, plaintext: &[u8]) -> String {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .cipher
            .encrypt(Nonce::from_slice(&nonce), plaintext)
            .expect("encryption does not fail for in-memory buffers");
        let mut out = nonce.to_vec();
        out.extend(ciphertext);
        STANDARD.encode(out)
    }

    pub fn open(&self, sealed: &str) -> Result<Vec<u8>> {
        let bytes = STANDARD
            .decode(sealed)
            .map_err(|_| anyhow!("sealed secret is not base64"))?;
        if bytes.len() <= NONCE_LEN {
            return Err(anyhow!("sealed secret is truncated"));
        }
        let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
        self.cipher
            .decrypt(Nonce::from_slice(nonce), ciphertext)
            .map_err(|_| {
                anyhow!("cannot decrypt stored secret (was HONE_QUANT_SECRET_KEY changed?)")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_tamper_detection() {
        let sb = SecretBox::new(&[7u8; 32]);
        let sealed = sb.seal(b"bot-token-123");
        assert_eq!(sb.open(&sealed).unwrap(), b"bot-token-123");
        assert_ne!(sb.seal(b"bot-token-123"), sealed, "nonces must differ");
        let other = SecretBox::new(&[8u8; 32]);
        assert!(other.open(&sealed).is_err());
        let mut bytes = STANDARD.decode(&sealed).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        assert!(sb.open(&STANDARD.encode(bytes)).is_err());
    }
}
