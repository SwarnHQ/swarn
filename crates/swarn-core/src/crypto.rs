//! The envelope layer (PRD §2.1–2.3): every frame between the daemon and a device is one HPKE seal,
//! signed by its sender and padded to a size bucket. No session state, so the same code opens a
//! live frame, a push blob or a mailbox frame. Pure functions; randomness is the only thing that
//! touches the OS.

use ed25519_dalek::Signer as _;
use hpke::{Deserializable, Kem as _, OpModeR, OpModeS, Serializable};
use p256::ecdsa::signature::Verifier as _;
use serde::{Deserialize, Serialize};

pub use {ed25519_dalek, p256};

type Kem = hpke::kem::X25519HkdfSha256;
type Kdf = hpke::kdf::HkdfSha256;
type Aead = hpke::aead::ChaCha20Poly1305;

pub const ENVELOPE_VERSION: u8 = 1;

/// A relay watching lengths learns only which bucket a frame fell into (§2.3). Anything bigger
/// than the last bucket is the session layer's job to chunk.
pub const BUCKETS: [usize; 5] = [512, 2048, 8192, 32768, 262144];

/// Domain separation: a frame sealed for one purpose can't be opened as another, so a captured
/// push blob can't be replayed as a live frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Session,
    Push,
    Mailbox,
}

impl Purpose {
    fn label(self) -> &'static [u8] {
        match self {
            Purpose::Session => b"session",
            Purpose::Push => b"push",
            Purpose::Mailbox => b"mailbox",
        }
    }

    fn info(self) -> Vec<u8> {
        [b"swarn/v1/".as_slice(), self.label()].concat()
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SealedFrame {
    pub v: u8,
    /// The recipient's static X25519 public key.
    #[serde(with = "serde_bytes")]
    pub to: Vec<u8>,
    /// HPKE encapsulated key.
    #[serde(with = "serde_bytes")]
    pub enc: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ct: Vec<u8>,
    /// Over `v || to || enc || ct`.
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
    /// The sender's signing public key: 32 bytes for Ed25519 (daemon), 33 or 65 for P-256 (device).
    #[serde(with = "serde_bytes")]
    pub from: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub pad: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CryptoError {
    BadKey,
    BadSignature,
    /// Wrong recipient, wrong purpose, or tampered ciphertext.
    CannotOpen,
    TooLarge,
    Encoding,
}

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CryptoError::BadKey => "malformed key",
            CryptoError::BadSignature => "signature does not verify",
            CryptoError::CannotOpen => "cannot open frame",
            CryptoError::TooLarge => "frame exceeds the largest size bucket",
            CryptoError::Encoding => "malformed frame",
        })
    }
}

impl std::error::Error for CryptoError {}

/// Whoever signs a frame: the daemon with Ed25519, a device with P-256 (Secure Enclave on the
/// phone; a software key in tests and simulated devices).
pub enum SigningKey {
    Ed25519(ed25519_dalek::SigningKey),
    P256(p256::ecdsa::SigningKey),
}

impl SigningKey {
    pub fn public(&self) -> Vec<u8> {
        match self {
            SigningKey::Ed25519(k) => k.verifying_key().to_bytes().to_vec(),
            SigningKey::P256(k) => p256_public(k),
        }
    }

    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        match self {
            SigningKey::Ed25519(k) => k.sign(message).to_bytes().to_vec(),
            SigningKey::P256(k) => {
                let sig: p256::ecdsa::Signature = k.sign(message);
                sig.to_der().as_bytes().to_vec()
            }
        }
    }
}

fn p256_public(key: &p256::ecdsa::SigningKey) -> Vec<u8> {
    key.verifying_key().to_sec1_point(true).as_bytes().to_vec()
}

/// Checks `sig` over `message` against a signing public key, telling the curve apart by length.
/// P-256 signatures may be DER (what the Secure Enclave returns) or raw 64 bytes.
pub fn verify(public: &[u8], message: &[u8], sig: &[u8]) -> Result<(), CryptoError> {
    match public.len() {
        32 => {
            let key = ed25519_dalek::VerifyingKey::from_bytes(
                public.try_into().map_err(|_| CryptoError::BadKey)?,
            )
            .map_err(|_| CryptoError::BadKey)?;
            let sig =
                ed25519_dalek::Signature::from_slice(sig).map_err(|_| CryptoError::BadSignature)?;
            key.verify_strict(message, &sig)
                .map_err(|_| CryptoError::BadSignature)
        }
        33 | 65 => {
            let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(public)
                .map_err(|_| CryptoError::BadKey)?;
            let sig = p256::ecdsa::Signature::from_der(sig)
                .or_else(|_| p256::ecdsa::Signature::from_slice(sig))
                .map_err(|_| CryptoError::BadSignature)?;
            key.verify(message, &sig)
                .map_err(|_| CryptoError::BadSignature)
        }
        _ => Err(CryptoError::BadKey),
    }
}

/// A fresh X25519 key pair as (secret, public) bytes.
pub fn x25519_generate() -> ([u8; 32], [u8; 32]) {
    let (secret, public) = Kem::gen_keypair();
    (to_32(&secret.to_bytes()), to_32(&public.to_bytes()))
}

pub fn x25519_public(secret: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
    let secret =
        <Kem as hpke::Kem>::PrivateKey::from_bytes(secret).map_err(|_| CryptoError::BadKey)?;
    Ok(to_32(&<Kem as hpke::Kem>::sk_to_pk(&secret).to_bytes()))
}

pub fn ed25519_generate() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&random_bytes::<32>())
}

pub fn p256_generate() -> p256::ecdsa::SigningKey {
    loop {
        // A random 32-byte string is a valid scalar with overwhelming probability.
        if let Ok(key) = p256::ecdsa::SigningKey::from_slice(&random_bytes::<32>()) {
            return key;
        }
    }
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("the OS random source works");
    out
}

fn to_32(bytes: &[u8]) -> [u8; 32] {
    bytes.try_into().expect("X25519 keys are 32 bytes")
}

fn signed_part(v: u8, to: &[u8], enc: &[u8], ct: &[u8]) -> Vec<u8> {
    [&[v][..], to, enc, ct].concat()
}

fn aad(from: &[u8], to: &[u8], purpose: Purpose) -> Vec<u8> {
    [from, to, purpose.label()].concat()
}

/// Seals `plaintext` to `recipient` for `purpose`, signs it, and pads the encoded frame to the
/// smallest bucket that fits.
pub fn seal(
    plaintext: &[u8],
    recipient: &[u8; 32],
    purpose: Purpose,
    signer: &SigningKey,
) -> Result<SealedFrame, CryptoError> {
    let pk =
        <Kem as hpke::Kem>::PublicKey::from_bytes(recipient).map_err(|_| CryptoError::BadKey)?;
    let from = signer.public();
    let (enc, ct) = hpke::single_shot_seal::<Aead, Kdf, Kem>(
        &OpModeS::Base,
        &pk,
        &purpose.info(),
        plaintext,
        &aad(&from, recipient, purpose),
    )
    .map_err(|_| CryptoError::CannotOpen)?;
    let enc = enc.to_bytes().to_vec();
    let sig = signer.sign(&signed_part(ENVELOPE_VERSION, recipient, &enc, &ct));
    let mut frame = SealedFrame {
        v: ENVELOPE_VERSION,
        to: recipient.to_vec(),
        enc,
        ct,
        sig,
        from,
        pad: Vec::new(),
    };
    frame.pad = vec![0; padding_for(encoded_len(&frame)?)?];
    Ok(frame)
}

/// Verifies the signature, then opens the frame. Returns the sender's signing key, which the
/// caller must check against the peer it expects: a valid signature from a stranger is still a
/// stranger.
pub fn open(
    frame: &SealedFrame,
    recipient_secret: &[u8; 32],
    purpose: Purpose,
) -> Result<(Vec<u8>, Vec<u8>), CryptoError> {
    if frame.v != ENVELOPE_VERSION {
        return Err(CryptoError::Encoding);
    }
    verify(
        &frame.from,
        &signed_part(frame.v, &frame.to, &frame.enc, &frame.ct),
        &frame.sig,
    )?;
    let sk = <Kem as hpke::Kem>::PrivateKey::from_bytes(recipient_secret)
        .map_err(|_| CryptoError::BadKey)?;
    let enc = <Kem as hpke::Kem>::EncappedKey::from_bytes(&frame.enc)
        .map_err(|_| CryptoError::Encoding)?;
    let plaintext = hpke::single_shot_open::<Aead, Kdf, Kem>(
        &OpModeR::Base,
        &sk,
        &enc,
        &purpose.info(),
        &frame.ct,
        &aad(&frame.from, &frame.to, purpose),
    )
    .map_err(|_| CryptoError::CannotOpen)?;
    Ok((frame.from.clone(), plaintext))
}

pub fn encode(frame: &SealedFrame) -> Result<Vec<u8>, CryptoError> {
    let mut out = Vec::new();
    ciborium::into_writer(frame, &mut out).map_err(|_| CryptoError::Encoding)?;
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<SealedFrame, CryptoError> {
    ciborium::from_reader(bytes).map_err(|_| CryptoError::Encoding)
}

fn encoded_len(frame: &SealedFrame) -> Result<usize, CryptoError> {
    encode(frame).map(|b| b.len())
}

/// How many zero bytes make a frame that encodes to `unpadded` bytes (with an empty pad) land
/// exactly on a bucket. A CBOR byte string's header grows with its length, so a few totals are
/// unreachable in one bucket; those move to the next.
fn padding_for(unpadded: usize) -> Result<usize, CryptoError> {
    let header = |n: usize| match n {
        0..=23 => 1,
        24..=255 => 2,
        256..=65535 => 3,
        _ => 5,
    };
    for bucket in BUCKETS.iter().filter(|b| **b >= unpadded) {
        let extra = bucket - unpadded;
        for grows in 0..=4 {
            if let Some(pad) = extra.checked_sub(grows)
                && header(pad) - 1 == grows
            {
                return Ok(pad);
            }
        }
    }
    Err(CryptoError::TooLarge)
}

/// What a device signs to approve or deny (§2.6): fields joined with the 0x1F unit separator, so
/// nobody can shuffle field boundaries.
pub fn approval_signing_input(
    request_id: &str,
    nonce: &str,
    decision: &str,
    device_id: &str,
    signed_at: &str,
) -> Vec<u8> {
    [
        "swarn/approval/v1",
        request_id,
        nonce,
        decision,
        device_id,
        signed_at,
    ]
    .join("\u{1f}")
    .into_bytes()
}

/// What a pairing device signs with its P-256 key (§2.5 step 4): proof that it holds the key, bound
/// to this pairing window and to the X25519 key it is registering.
pub fn pairing_signing_input(
    pairing_token: &str,
    device_id: &str,
    x25519_public_b64: &str,
) -> Vec<u8> {
    ["swarn/pair/v1", pairing_token, device_id, x25519_public_b64]
        .join("\u{1f}")
        .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_sender_identity() {
        let (secret, public) = x25519_generate();
        let daemon = SigningKey::Ed25519(ed25519_generate());
        let frame = seal(b"hello", &public, Purpose::Session, &daemon).unwrap();
        let (from, text) = open(
            &decode(&encode(&frame).unwrap()).unwrap(),
            &secret,
            Purpose::Session,
        )
        .unwrap();
        assert_eq!(text, b"hello");
        assert_eq!(from, daemon.public());

        let device = SigningKey::P256(p256_generate());
        let frame = seal(b"from the phone", &public, Purpose::Push, &device).unwrap();
        assert_eq!(
            open(&frame, &secret, Purpose::Push).unwrap().0,
            device.public()
        );
    }

    #[test]
    fn wrong_purpose_wrong_key_or_tampering_fails() {
        let (secret, public) = x25519_generate();
        let signer = SigningKey::Ed25519(ed25519_generate());
        let frame = seal(b"hello", &public, Purpose::Push, &signer).unwrap();
        assert_eq!(
            open(&frame, &secret, Purpose::Session),
            Err(CryptoError::CannotOpen)
        );
        let (other, _) = x25519_generate();
        assert_eq!(
            open(&frame, &other, Purpose::Push),
            Err(CryptoError::CannotOpen)
        );
        let mut tampered = frame.clone();
        tampered.ct[0] ^= 1;
        assert_eq!(
            open(&tampered, &secret, Purpose::Push),
            Err(CryptoError::BadSignature)
        );
        let mut forged = frame;
        forged.from = SigningKey::Ed25519(ed25519_generate()).public();
        assert_eq!(
            open(&forged, &secret, Purpose::Push),
            Err(CryptoError::BadSignature)
        );
    }

    #[test]
    fn every_frame_lands_on_a_bucket() {
        let (_, public) = x25519_generate();
        let signers = [
            SigningKey::Ed25519(ed25519_generate()),
            SigningKey::P256(p256_generate()),
        ];
        // Every plaintext length near each bucket edge, plus a spread in between.
        let lengths = (0..600)
            .chain(1800..2100)
            .chain([5000, 8000, 30000, 200_000]);
        for len in lengths {
            for signer in &signers {
                let frame = seal(&vec![7u8; len], &public, Purpose::Session, signer).unwrap();
                let size = encode(&frame).unwrap().len();
                assert!(
                    BUCKETS.contains(&size),
                    "{len} byte plaintext encoded to {size}"
                );
            }
        }
        assert_eq!(
            seal(&vec![0; 300_000], &public, Purpose::Session, &signers[0]),
            Err(CryptoError::TooLarge)
        );
    }

    /// A frame sealed once and committed. If a dependency upgrade changes the suite, the key
    /// derivation or the encoding, this stops opening and the test fails loudly (§9.1).
    #[test]
    fn committed_vector_still_opens() {
        let vector: serde_json::Value =
            serde_json::from_str(include_str!("../tests/vectors.json")).unwrap();
        let hex = |s: &str| {
            (0..s.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
                .collect::<Vec<u8>>()
        };
        let secret: [u8; 32] = hex(vector["recipient_secret"].as_str().unwrap())
            .try_into()
            .unwrap();
        let frame = decode(&hex(vector["frame"].as_str().unwrap())).unwrap();
        let (from, text) = open(&frame, &secret, Purpose::Session).unwrap();
        assert_eq!(text, vector["plaintext"].as_str().unwrap().as_bytes());
        assert_eq!(from, hex(vector["sender"].as_str().unwrap()));
    }

    #[test]
    fn signing_inputs_are_exactly_this() {
        assert_eq!(
            approval_signing_input(
                "01JA7",
                "bm9uY2U",
                "allow_once",
                "d1",
                "2026-10-15T14:00:00Z"
            ),
            b"swarn/approval/v1\x1f01JA7\x1fbm9uY2U\x1fallow_once\x1fd1\x1f2026-10-15T14:00:00Z"
                .to_vec()
        );
        assert_eq!(
            pairing_signing_input("pw", "d1", "x"),
            b"swarn/pair/v1\x1fpw\x1fd1\x1fx".to_vec()
        );
    }
}
