use anyhow::Result;
use ed25519_dalek::{Keypair, PublicKey, SecretKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPair {
    public: PublicKey,
    secret: SecretKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let mut csprng = OsRng;
        let keypair = Keypair::generate(&mut csprng);
        
        Ok(KeyPair {
            public: keypair.public,
            secret: keypair.secret,
        })
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public
    }
}

pub struct CryptoService {
    keypair: KeyPair,
}

impl CryptoService {
    pub fn new() -> Result<Self> {
        Ok(Self {
            keypair: KeyPair::generate()?,
        })
    }

    pub fn encrypt_message(&self, message: &[u8], recipient_public_key: &PublicKey) -> Result<Vec<u8>> {
        // TODO: Implement message encryption using X25519 key exchange
        // and ChaCha20-Poly1305 AEAD
        unimplemented!()
    }

    pub fn decrypt_message(&self, encrypted_message: &[u8], sender_public_key: &PublicKey) -> Result<Vec<u8>> {
        // TODO: Implement message decryption
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_pair_generation() {
        let keypair = KeyPair::generate().unwrap();
        assert!(keypair.public_key().to_bytes().len() == 32);
    }
} 