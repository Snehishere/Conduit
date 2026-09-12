use chacha20poly1305::{
    aead::{Aead, KeyInit, OsRng},
    XChaCha20Poly1305, XNonce,
};
use rand::RngCore;
use std::sync::Arc;
use x25519_dalek::{PublicKey, StaticSecret};

#[derive(Clone)]
pub struct EncryptionManager {
    private_key: Arc<StaticSecret>,
    public_key: Arc<PublicKey>,
}

impl EncryptionManager {
    pub fn new() -> Self {
        let private_key = StaticSecret::random_from_rng(OsRng);
        let public_key = PublicKey::from(&private_key);

        EncryptionManager {
            private_key: Arc::new(private_key),
            public_key: Arc::new(public_key),
        }
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.as_bytes())
    }

    pub fn derive_shared_secret(&self, peer_public_key_hex: &str) -> Result<[u8; 32], String> {
        let peer_bytes = hex::decode(peer_public_key_hex)
            .map_err(|e| format!("Invalid public key hex: {}", e))?;
        if peer_bytes.len() != 32 {
            return Err(format!(
                "Invalid public key length: expected 32 bytes, got {}",
                peer_bytes.len()
            ));
        }
        let mut peer_key_bytes = [0u8; 32];
        peer_key_bytes.copy_from_slice(&peer_bytes);

        let peer_public = PublicKey::from(peer_key_bytes);
        let shared_secret = self.private_key.diffie_hellman(&peer_public);
        Ok(*shared_secret.as_bytes())
    }

    pub fn encrypt(&self, shared_secret: &[u8; 32], plaintext: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
        let cipher = XChaCha20Poly1305::new(shared_secret.into());

        let mut nonce_bytes = [0u8; 24];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = XNonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|e| format!("Encryption failed: {}", e))?;

        Ok((nonce_bytes.to_vec(), ciphertext))
    }

    pub fn decrypt(
        &self,
        shared_secret: &[u8; 32],
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<String, String> {
        let cipher = XChaCha20Poly1305::new(shared_secret.into());
        let nonce = XNonce::from_slice(nonce);

        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("Decryption failed: {}", e))?;

        String::from_utf8(plaintext).map_err(|e| format!("Invalid UTF-8: {}", e))
    }
}

pub fn generate_token_hex() -> String {
    let mut token = [0u8; 32];
    OsRng.fill_bytes(&mut token);
    hex::encode(token)
}
