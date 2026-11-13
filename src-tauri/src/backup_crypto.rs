//! Optional `age` encryption of backup files (X25519 recipients only).
//!
//! Encrypted backups are the binary age format wrapping the gzipped JSON, so
//! `age -d -i key.txt badger-YYYY-MM-DD.json.gz.age | gunzip` reads them without Badger.
//! Secret keys are never stored, logged or echoed in error messages.

use std::str::FromStr;

use age::secrecy::ExposeSecret;

use crate::AppError;

/// First bytes of every binary age file.
pub const AGE_MAGIC: &[u8] = b"age-encryption.org/v1\n";

const SECRET_KEY_PREFIX: &str = "AGE-SECRET-KEY-1";
const INVALID_PUBLIC_KEY: &str = "Not a valid age public key (it starts with age1)";
const INVALID_SECRET_KEY: &str = "Not a valid age secret key (it starts with AGE-SECRET-KEY-1)";
pub const WRONG_KEY: &str = "This key can't decrypt this backup";

/// A freshly generated key pair. The secret is shown once and never stored.
#[derive(Debug, serde::Serialize)]
pub struct GeneratedKey {
    pub public_key: String,
    pub secret_key: String,
}

pub fn is_encrypted(bytes: &[u8]) -> bool {
    bytes.starts_with(AGE_MAGIC)
}

pub fn generate_key() -> GeneratedKey {
    let identity = age::x25519::Identity::generate();
    GeneratedKey {
        public_key: identity.to_public().to_string(),
        secret_key: identity.to_string().expose_secret().to_string(),
    }
}

/// Parses an `age1…` public key, ignoring surrounding whitespace.
pub fn parse_public_key(input: &str) -> Result<age::x25519::Recipient, AppError> {
    age::x25519::Recipient::from_str(input.trim())
        .map_err(|_| AppError::Custom(INVALID_PUBLIC_KEY.to_string()))
}

/// Parses a secret key: a bare `AGE-SECRET-KEY-1…` or the text of an age key file
/// (the first secret-key line wins; comments and blank lines are ignored).
pub fn parse_secret_key(input: &str) -> Result<age::x25519::Identity, AppError> {
    input
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with(SECRET_KEY_PREFIX))
        .and_then(|line| age::x25519::Identity::from_str(line).ok())
        .ok_or_else(|| AppError::Custom(INVALID_SECRET_KEY.to_string()))
}

/// Checks that `secret_key` belongs to `public_key`.
pub fn verify_key_pair(public_key: &str, secret_key: &str) -> Result<(), AppError> {
    let recipient = parse_public_key(public_key)?;
    let identity = parse_secret_key(secret_key)?;
    if identity.to_public().to_string() == recipient.to_string() {
        Ok(())
    } else {
        Err(AppError::Custom(
            "This secret key doesn't match the public key".to_string(),
        ))
    }
}

pub fn encrypt(public_key: &str, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    let recipient = parse_public_key(public_key)?;
    age::encrypt(&recipient, plaintext)
        .map_err(|e| AppError::Custom(format!("Could not encrypt the backup: {e}")))
}

pub fn decrypt(secret_key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
    let identity = parse_secret_key(secret_key)?;
    age::decrypt(&identity, ciphertext).map_err(|e| match e {
        age::DecryptError::NoMatchingKeys => AppError::Custom(WRONG_KEY.to_string()),
        _ => AppError::Custom("Backup file is not a valid Badger backup".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_round_trips_and_verifies() {
        let key = generate_key();
        assert!(key.public_key.starts_with("age1"));
        assert!(key.secret_key.starts_with(SECRET_KEY_PREFIX));
        verify_key_pair(&key.public_key, &key.secret_key).expect("pair should verify");

        let ciphertext = encrypt(&key.public_key, b"hello").expect("encrypt");
        assert!(is_encrypted(&ciphertext));
        assert_eq!(
            decrypt(&key.secret_key, &ciphertext).expect("decrypt"),
            b"hello"
        );
    }

    #[test]
    fn wrong_or_truncated_secret_is_rejected() {
        let key = generate_key();
        let other = generate_key();
        assert!(verify_key_pair(&key.public_key, &other.secret_key).is_err());
        let truncated = &key.secret_key[..key.secret_key.len() - 4];
        assert!(verify_key_pair(&key.public_key, truncated).is_err());

        let ciphertext = encrypt(&key.public_key, b"x").expect("encrypt");
        let err = decrypt(&other.secret_key, &ciphertext).unwrap_err();
        assert_eq!(err.to_string(), WRONG_KEY);
    }

    #[test]
    fn secret_key_accepts_whitespace_and_key_file_text() {
        let key = generate_key();
        let file = format!(
            "# created: 2026-11-12T10:00:00+01:00\n# public key: {}\n  {}  \n",
            key.public_key, key.secret_key
        );
        verify_key_pair(&format!("  {}\n", key.public_key), &file).expect("key file parses");
    }

    #[test]
    fn invalid_keys_get_user_messages_without_echoing_input() {
        let err = parse_public_key("age1nope").unwrap_err().to_string();
        assert_eq!(err, INVALID_PUBLIC_KEY);
        let err = parse_secret_key("AGE-SECRET-KEY-1SECRETSTUFF")
            .err()
            .unwrap()
            .to_string();
        assert_eq!(err, INVALID_SECRET_KEY);
        assert!(!err.contains("SECRETSTUFF"));
    }

    #[test]
    fn plain_and_gzip_bytes_are_not_encrypted() {
        assert!(!is_encrypted(b"{}"));
        assert!(!is_encrypted(&[0x1f, 0x8b, 0x08]));
    }
}
