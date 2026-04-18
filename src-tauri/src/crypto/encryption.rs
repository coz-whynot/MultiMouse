use x25519_dalek::{EphemeralSecret, PublicKey};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use chacha20poly1305::aead::{Aead, KeyInit};
use hkdf::Hkdf;
use sha2::Sha256;
use rand::rngs::OsRng;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const NONCE_LEN: usize = 12;

/// One-directional encryptor/decryptor. Each send uses an incrementing counter nonce
/// embedded in the ciphertext (first 12 bytes). Decryption extracts the nonce from data.
pub struct Channel {
    cipher: ChaCha20Poly1305,
    nonce_ctr: u64,
}

impl Channel {
    fn new(key: [u8; 32]) -> Self {
        Self { cipher: ChaCha20Poly1305::new(Key::from_slice(&key)), nonce_ctr: 0 }
    }

    /// Encrypt plaintext. Output = 12-byte-nonce + ciphertext + 16-byte-tag.
    pub fn seal(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        nonce_bytes[..8].copy_from_slice(&self.nonce_ctr.to_le_bytes());
        self.nonce_ctr += 1;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let mut out = nonce_bytes.to_vec();
        out.extend(self.cipher.encrypt(nonce, plaintext).expect("encrypt"));
        out
    }

    /// Decrypt. Input must be nonce(12) + ciphertext + tag(16).
    pub fn open(&self, data: &[u8]) -> Option<Vec<u8>> {
        if data.len() < NONCE_LEN { return None; }
        let nonce = Nonce::from_slice(&data[..NONCE_LEN]);
        self.cipher.decrypt(nonce, &data[NONCE_LEN..]).ok()
    }
}

fn derive_pair(shared: &[u8]) -> ([u8; 32], [u8; 32]) {
    let hk = Hkdf::<Sha256>::new(None, shared);
    let mut c2s = [0u8; 32];
    let mut s2c = [0u8; 32];
    hk.expand(b"multimouse-c2s-v2", &mut c2s).unwrap();
    hk.expand(b"multimouse-s2c-v2", &mut s2c).unwrap();
    (c2s, s2c)
}

/// Derive a 6-digit Short Authentication String (SAS) from the shared secret.
/// If a MitM is active, each side's shared secret differs — the SAS will mismatch
/// and users can visually detect the attack.
fn derive_sas(shared: &[u8]) -> String {
    let hk = Hkdf::<Sha256>::new(None, shared);
    let mut bytes = [0u8; 4];
    hk.expand(b"multimouse-sas-v1", &mut bytes).unwrap();
    let n = u32::from_be_bytes(bytes) % 1_000_000;
    format!("{:06}", n)
}

/// Result of a successful handshake: two channels + the visual-verification PIN.
pub struct Handshake {
    pub send: Channel,
    pub recv: Channel,
    pub sas: String,
}

/// Server-side handshake: read client pubkey, send server pubkey.
pub async fn server_handshake(stream: &mut tokio::net::TcpStream) -> Option<Handshake> {
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        async {
            let mut client_pub_bytes = [0u8; 32];
            stream.read_exact(&mut client_pub_bytes).await.ok()?;
            let client_pub = PublicKey::from(client_pub_bytes);

            let server_secret = EphemeralSecret::random_from_rng(OsRng);
            let server_pub = PublicKey::from(&server_secret);
            stream.write_all(server_pub.as_bytes()).await.ok()?;
            stream.flush().await.ok()?;

            let shared = server_secret.diffie_hellman(&client_pub);
            let (c2s, s2c) = derive_pair(shared.as_bytes());
            let sas = derive_sas(shared.as_bytes());
            Some(Handshake {
                send: Channel::new(s2c),
                recv: Channel::new(c2s),
                sas,
            })
        }
    ).await;
    result.ok().flatten()
}

/// Client-side handshake: send client pubkey, read server pubkey.
pub async fn client_handshake(stream: &mut tokio::net::TcpStream) -> Option<Handshake> {
    let result = tokio::time::timeout(
        tokio::time::Duration::from_secs(10),
        async {
            let client_secret = EphemeralSecret::random_from_rng(OsRng);
            let client_pub = PublicKey::from(&client_secret);
            stream.write_all(client_pub.as_bytes()).await.ok()?;
            stream.flush().await.ok()?;

            let mut server_pub_bytes = [0u8; 32];
            stream.read_exact(&mut server_pub_bytes).await.ok()?;
            let server_pub = PublicKey::from(server_pub_bytes);

            let shared = client_secret.diffie_hellman(&server_pub);
            let (c2s, s2c) = derive_pair(shared.as_bytes());
            let sas = derive_sas(shared.as_bytes());
            Some(Handshake {
                send: Channel::new(c2s),
                recv: Channel::new(s2c),
                sas,
            })
        }
    ).await;
    result.ok().flatten()
}
