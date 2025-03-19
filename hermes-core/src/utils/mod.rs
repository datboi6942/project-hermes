use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub fn generate_random_bytes(length: usize) -> Result<Vec<u8>> {
    use rand::RngCore;
    let mut bytes = vec![0u8; length];
    rand::thread_rng().fill_bytes(&mut bytes);
    Ok(bytes)
}

pub fn secure_compare(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    
    let mut result = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        result |= x ^ y;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secure_compare() {
        let a = b"hello";
        let b = b"hello";
        let c = b"world";
        
        assert!(secure_compare(a, b));
        assert!(!secure_compare(a, c));
    }

    #[test]
    fn test_generate_random_bytes() {
        let bytes = generate_random_bytes(32).unwrap();
        assert_eq!(bytes.len(), 32);
        
        // Test that bytes are not all zeros
        let sum: u8 = bytes.iter().sum();
        assert_ne!(sum, 0);
    }
} 