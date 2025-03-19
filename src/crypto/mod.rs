use anyhow::{Result, Context};
use ed25519_dalek::{Keypair, PublicKey, SecretKey, Signature};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305,
    Key,
    Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::convert::TryFrom;

const KEY_INFO: &[u8] = b"Project Hermes Key Derivation";
const NONCE_INFO: &[u8] = b"Project Hermes Nonce Derivation";
const SESSION_KEY_INFO: &[u8] = b"Project Hermes Session Key";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPair {
    public: PublicKey,
    secret: SecretKey,
    x25519_public: X25519PublicKey,
    x25519_secret: StaticSecret,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let mut csprng = OsRng;
        
        // Generate Ed25519 keypair for signatures
        let ed25519_keypair = Keypair::generate(&mut csprng);
        
        // Generate X25519 keypair for key exchange
        let x25519_secret = StaticSecret::new(&mut csprng);
        let x25519_public = X25519PublicKey::from(&x25519_secret);
        
        Ok(KeyPair {
            public: ed25519_keypair.public,
            secret: ed25519_keypair.secret,
            x25519_public,
            x25519_secret,
        })
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public
    }

    pub fn x25519_public_key(&self) -> &X25519PublicKey {
        &self.x25519_public
    }

    pub fn sign(&self, message: &[u8]) -> Result<Signature> {
        let keypair = Keypair {
            public: self.public.clone(),
            secret: self.secret.clone(),
        };
        
        Ok(keypair.sign(message))
    }
}

#[derive(Debug)]
pub struct CryptoService {
    keypair: KeyPair,
    cipher: ChaCha20Poly1305,
}

impl CryptoService {
    pub fn new() -> Result<Self> {
        let keypair = KeyPair::generate()?;
        
        // Initialize ChaCha20-Poly1305 cipher with a random key
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
        
        Ok(Self {
            keypair,
            cipher,
        })
    }

    fn derive_keys(&self, shared_secret: &[u8]) -> Result<(Key, Nonce)> {
        // Create HKDF instance
        let hkdf = Hkdf::<Sha256>::new(None, shared_secret);
        
        // Derive encryption key
        let mut key_bytes = [0u8; 32];
        hkdf.expand(KEY_INFO, &mut key_bytes)
            .context("Failed to derive encryption key")?;
        
        // Derive nonce
        let mut nonce_bytes = [0u8; 12];
        hkdf.expand(NONCE_INFO, &mut nonce_bytes)
            .context("Failed to derive nonce")?;
        
        Ok((
            Key::from_slice(&key_bytes),
            Nonce::from_slice(&nonce_bytes)
        ))
    }

    pub fn encrypt_message(&self, message: &[u8], recipient_public_key: &PublicKey) -> Result<Vec<u8>> {
        // Generate ephemeral key pair for this message
        let mut csprng = OsRng;
        let ephemeral_secret = StaticSecret::new(&mut csprng);
        let ephemeral_public = X25519PublicKey::from(&ephemeral_secret);

        // Perform key exchange
        let shared_secret = ephemeral_secret.diffie_hellman(&self.keypair.x25519_public_key());
        
        // Derive encryption key and nonce using HKDF
        let (key, nonce) = self.derive_keys(&shared_secret.to_bytes())?;

        // Create new cipher instance with derived key
        let cipher = ChaCha20Poly1305::new(key);

        // Encrypt message
        let ciphertext = cipher
            .encrypt(&nonce, message)
            .context("Failed to encrypt message")?;

        // Sign the encrypted message
        let signature = self.sign_message(&ciphertext)?;

        // Combine ephemeral public key, signature, and ciphertext
        let mut result = Vec::with_capacity(32 + 64 + ciphertext.len());
        result.extend_from_slice(&ephemeral_public.to_bytes());
        result.extend_from_slice(&signature);
        result.extend_from_slice(&ciphertext);

        Ok(result)
    }

    pub fn decrypt_message(&self, encrypted_message: &[u8], sender_public_key: &PublicKey) -> Result<Vec<u8>> {
        if encrypted_message.len() < 96 { // 32 bytes for public key + 64 bytes for signature
            return Err(anyhow::anyhow!("Invalid encrypted message length"));
        }

        // Extract components
        let ephemeral_public = X25519PublicKey::from_slice(&encrypted_message[..32])
            .context("Failed to parse ephemeral public key")?;
        let signature = &encrypted_message[32..96];
        let ciphertext = &encrypted_message[96..];

        // Verify signature
        if !self.verify_signature(ciphertext, signature, sender_public_key)? {
            return Err(anyhow::anyhow!("Invalid message signature"));
        }

        // Perform key exchange
        let shared_secret = self.keypair.x25519_secret.diffie_hellman(&ephemeral_public);
        
        // Derive encryption key and nonce using HKDF
        let (key, nonce) = self.derive_keys(&shared_secret.to_bytes())?;

        // Create new cipher instance with derived key
        let cipher = ChaCha20Poly1305::new(key);

        // Decrypt message
        let plaintext = cipher
            .decrypt(&nonce, ciphertext)
            .context("Failed to decrypt message")?;

        Ok(plaintext)
    }

    pub fn sign_message(&self, message: &[u8]) -> Result<Vec<u8>> {
        let signature = self.keypair.sign(message)?;
        Ok(signature.to_bytes().to_vec())
    }

    pub fn verify_signature(&self, message: &[u8], signature: &[u8], public_key: &PublicKey) -> Result<bool> {
        let signature = Signature::from_bytes(signature)
            .context("Failed to parse signature")?;
        
        Ok(public_key.verify(message, &signature).is_ok())
    }

    pub fn generate_session_key(&self) -> Result<Vec<u8>> {
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        
        // Use HKDF to derive a session key
        let hkdf = Hkdf::<Sha256>::new(None, &key_bytes);
        let mut session_key = [0u8; 32];
        hkdf.expand(SESSION_KEY_INFO, &mut session_key)
            .context("Failed to derive session key")?;
        
        Ok(session_key.to_vec())
    }

    pub fn encrypt_with_session_key(&self, message: &[u8], session_key: &[u8]) -> Result<Vec<u8>> {
        let key = Key::from_slice(session_key);
        let cipher = ChaCha20Poly1305::new(key);
        
        // Generate random nonce
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        // Encrypt message
        let ciphertext = cipher
            .encrypt(&nonce, message)
            .context("Failed to encrypt message with session key")?;
        
        // Combine nonce and ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }

    pub fn decrypt_with_session_key(&self, encrypted_message: &[u8], session_key: &[u8]) -> Result<Vec<u8>> {
        if encrypted_message.len() < 12 {
            return Err(anyhow::anyhow!("Invalid encrypted message length"));
        }
        
        let key = Key::from_slice(session_key);
        let cipher = ChaCha20Poly1305::new(key);
        
        // Extract nonce and ciphertext
        let nonce = Nonce::from_slice(&encrypted_message[..12]);
        let ciphertext = &encrypted_message[12..];
        
        // Decrypt message
        let plaintext = cipher
            .decrypt(&nonce, ciphertext)
            .context("Failed to decrypt message with session key")?;
        
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_pair_generation() {
        let keypair = KeyPair::generate().unwrap();
        assert!(keypair.public_key().to_bytes().len() == 32);
        assert!(keypair.x25519_public_key().to_bytes().len() == 32);
    }

    #[test]
    fn test_message_encryption_decryption() {
        let crypto_service = CryptoService::new().unwrap();
        let recipient_keypair = KeyPair::generate().unwrap();
        
        let message = b"Hello, secure world!";
        let encrypted = crypto_service
            .encrypt_message(message, recipient_keypair.public_key())
            .unwrap();
        
        let decrypted = crypto_service
            .decrypt_message(&encrypted, recipient_keypair.public_key())
            .unwrap();
        
        assert_eq!(message.to_vec(), decrypted);
    }

    #[test]
    fn test_message_signing_verification() {
        let crypto_service = CryptoService::new().unwrap();
        let message = b"Test message for signing";
        
        let signature = crypto_service.sign_message(message).unwrap();
        assert_eq!(signature.len(), 64); // Ed25519 signatures are 64 bytes
        
        let is_valid = crypto_service
            .verify_signature(message, &signature, crypto_service.keypair.public_key())
            .unwrap();
        assert!(is_valid);
        
        // Test with modified message
        let modified_message = b"Modified message";
        let is_valid = crypto_service
            .verify_signature(modified_message, &signature, crypto_service.keypair.public_key())
            .unwrap();
        assert!(!is_valid);
    }

    #[test]
    fn test_key_derivation() {
        let crypto_service = CryptoService::new().unwrap();
        let shared_secret = [0u8; 32];
        
        let (key1, nonce1) = crypto_service.derive_keys(&shared_secret).unwrap();
        let (key2, nonce2) = crypto_service.derive_keys(&shared_secret).unwrap();
        
        // Same input should produce same output
        assert_eq!(key1.as_slice(), key2.as_slice());
        assert_eq!(nonce1.as_slice(), nonce2.as_slice());
        
        // Different input should produce different output
        let different_secret = [1u8; 32];
        let (key3, nonce3) = crypto_service.derive_keys(&different_secret).unwrap();
        assert_ne!(key1.as_slice(), key3.as_slice());
        assert_ne!(nonce1.as_slice(), nonce3.as_slice());
    }

    #[test]
    fn test_session_key_encryption() {
        let crypto_service = CryptoService::new().unwrap();
        let session_key = crypto_service.generate_session_key().unwrap();
        
        let message = b"Test message for session key encryption";
        let encrypted = crypto_service
            .encrypt_with_session_key(message, &session_key)
            .unwrap();
        
        let decrypted = crypto_service
            .decrypt_with_session_key(&encrypted, &session_key)
            .unwrap();
        
        assert_eq!(message.to_vec(), decrypted);
    }
} 