use anyhow::Result;
use ed25519_dalek::Signer;
use rand::rngs::OsRng;
use rand::RngCore;
use x25519_dalek::PublicKey as X25519PublicKey;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305,
    Key,
    Nonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use std::convert::TryInto;

// Using type aliases to make updates easier
type Ed25519PublicKey = ed25519_dalek::VerifyingKey;
type Ed25519SecretKey = ed25519_dalek::SigningKey;
type Ed25519Signature = ed25519_dalek::Signature;

const KEY_INFO: &[u8] = b"Project Hermes Key Derivation";
const NONCE_INFO: &[u8] = b"Project Hermes Nonce Derivation";
const SESSION_KEY_INFO: &[u8] = b"Project Hermes Session Key";

#[derive(Debug, Clone)]
pub struct KeyPair {
    pub public: Ed25519PublicKey,
    secret: Ed25519SecretKey,
    pub x25519_public: X25519PublicKey,
}

impl KeyPair {
    pub fn generate() -> Result<Self> {
        let mut csprng = OsRng;
        
        // Generate Ed25519 keypair for signatures
        let ed25519_secret = Ed25519SecretKey::generate(&mut csprng);
        let ed25519_public = ed25519_secret.verifying_key();
        
        // Generate X25519 keypair for key exchange (if needed)
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        let x25519_public = X25519PublicKey::from(seed);
        
        Ok(KeyPair {
            public: ed25519_public,
            secret: ed25519_secret,
            x25519_public,
        })
    }

    pub fn public_key(&self) -> &Ed25519PublicKey {
        &self.public
    }

    pub fn x25519_public_key(&self) -> &X25519PublicKey {
        &self.x25519_public
    }

    pub fn sign(&self, message: &[u8]) -> Result<Ed25519Signature> {
        Ok(self.secret.sign(message))
    }
}

#[derive(Debug, Clone)]
pub struct CryptoService {
    keypair: KeyPair,
}

impl CryptoService {
    pub fn new() -> Result<Self> {
        let keypair = KeyPair::generate()?;
        
        Ok(CryptoService {
            keypair,
        })
    }

    pub fn generate_key_pair() -> Result<KeyPair> {
        KeyPair::generate()
    }

    pub fn derive_key_and_nonce(&self, shared_secret: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let hkdf = Hkdf::<Sha256>::new(None, shared_secret);
        
        let mut key_bytes = vec![0u8; 32]; // ChaCha20Poly1305 key size
        let mut nonce_bytes = vec![0u8; 12]; // ChaCha20Poly1305 nonce size
        
        hkdf.expand(KEY_INFO, &mut key_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to derive encryption key: {:?}", e))?;
        
        hkdf.expand(NONCE_INFO, &mut nonce_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to derive nonce: {:?}", e))?;
        
        Ok((key_bytes, nonce_bytes))
    }

    pub fn encrypt_message(&self, message: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        let key = Key::from_slice(key);
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let cipher = ChaCha20Poly1305::new(key);
        
        let ciphertext = cipher
            .encrypt(nonce, message)
            .map_err(|e| anyhow::anyhow!("Failed to encrypt message: {:?}", e))?;
        
        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }

    pub fn decrypt_message(&self, encrypted_message: &[u8], key: &[u8]) -> Result<Vec<u8>> {
        if encrypted_message.len() < 12 {
            return Err(anyhow::anyhow!("Invalid encrypted message format"));
        }
        
        let nonce_bytes = &encrypted_message[..12];
        let ciphertext = &encrypted_message[12..];
        
        let key = Key::from_slice(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        
        let cipher = ChaCha20Poly1305::new(key);
        
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Failed to decrypt message: {:?}", e))?;
        
        Ok(plaintext)
    }

    pub fn verify_signature(&self, public_key: &Ed25519PublicKey, message: &[u8], signature: &[u8]) -> Result<bool> {
        if signature.len() != 64 {
            return Err(anyhow::anyhow!("Invalid signature length"));
        }
        
        let signature_bytes: [u8; 64] = signature.try_into()
            .map_err(|_| anyhow::anyhow!("Failed to parse signature"))?;
            
        let signature = Ed25519Signature::from_bytes(&signature_bytes);
        
        match public_key.verify_strict(message, &signature) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    pub fn generate_session_key(&self) -> Result<Vec<u8>> {
        let mut key_bytes = vec![0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        
        let hkdf = Hkdf::<Sha256>::new(None, &key_bytes);
        let mut session_key = vec![0u8; 32];
        
        hkdf.expand(SESSION_KEY_INFO, &mut session_key)
            .map_err(|e| anyhow::anyhow!("Failed to derive session key: {:?}", e))?;
            
        Ok(session_key)
    }

    pub fn encrypt_with_session_key(&self, message: &[u8], session_key: &[u8]) -> Result<Vec<u8>> {
        let key = Key::from_slice(session_key);
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        
        let cipher = ChaCha20Poly1305::new(key);
        
        let ciphertext = cipher
            .encrypt(nonce, message)
            .map_err(|e| anyhow::anyhow!("Failed to encrypt message with session key: {:?}", e))?;
        
        // Prepend nonce to ciphertext
        let mut result = Vec::with_capacity(nonce_bytes.len() + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }

    pub fn decrypt_with_session_key(&self, encrypted_message: &[u8], session_key: &[u8]) -> Result<Vec<u8>> {
        if encrypted_message.len() < 12 {
            return Err(anyhow::anyhow!("Invalid encrypted message format"));
        }
        
        let nonce_bytes = &encrypted_message[..12];
        let ciphertext = &encrypted_message[12..];
        
        let key = Key::from_slice(session_key);
        let nonce = Nonce::from_slice(nonce_bytes);
        
        let cipher = ChaCha20Poly1305::new(key);
        
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("Failed to decrypt message with session key: {:?}", e))?;
        
        Ok(plaintext)
    }

    // Returns the public key bytes for sharing with other nodes
    pub fn get_public_key(&self) -> Vec<u8> {
        self.keypair.public.to_bytes().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_pair_generation() {
        let key_pair = KeyPair::generate().unwrap();
        assert_eq!(key_pair.public.to_bytes().len(), 32);
    }

    #[test]
    fn test_signing_and_verification() {
        let key_pair = KeyPair::generate().unwrap();
        let message = b"Test message";
        
        let signature = key_pair.sign(message).unwrap();
        
        let crypto_service = CryptoService::new().unwrap();
        let verified = crypto_service.verify_signature(&key_pair.public, message, signature.to_bytes().as_ref()).unwrap();
        
        assert!(verified);
    }

    #[test]
    fn test_encryption_and_decryption() {
        let crypto_service = CryptoService::new().unwrap();
        let message = b"Secret message";
        let session_key = crypto_service.generate_session_key().unwrap();
        
        let encrypted = crypto_service.encrypt_with_session_key(message, &session_key).unwrap();
        let decrypted = crypto_service.decrypt_with_session_key(&encrypted, &session_key).unwrap();
        
        assert_eq!(decrypted, message);
    }
} 