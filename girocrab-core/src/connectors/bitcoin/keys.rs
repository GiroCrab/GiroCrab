//! Watch-only key handling: parse an extended public key (xpub / ypub / zpub),
//! derive child public keys without any private key (BIP32 `CKDpub`) and turn
//! them into mainnet addresses (P2PKH, P2SH-P2WPKH, P2WPKH). No secret ever
//! enters this module: an extended public key can only watch, never spend.

use hmac::{Hmac, KeyInit, Mac};
use k256::elliptic_curve::PrimeField;
use k256::elliptic_curve::sec1::{FromSec1Point, ToSec1Point};
use k256::{AffinePoint, ProjectivePoint, Scalar, Sec1Point};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256, Sha512};

/// How the wallet turns a public key into an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    /// BIP44 legacy `1…`
    P2pkh,
    /// BIP49 nested SegWit `3…`
    P2shP2wpkh,
    /// BIP84 native SegWit `bc1q…`
    P2wpkh,
}

/// A parsed extended public key (account level, e.g. `m/84'/0'/0'`).
#[derive(Clone, PartialEq, Eq)]
pub struct ExtendedPubKey {
    pub chain_code: [u8; 32],
    pub key: [u8; 33],
    /// Script kinds to scan: one for ypub/zpub, legacy and native SegWit for an
    /// xpub (several wallets export SegWit accounts as xpub).
    pub kinds: Vec<ScriptKind>,
    /// BIP32 depth: 3 = account (`m/84'/0'/0'`), 4 = one chain of it, 0 = master.
    pub depth: u8,
}

impl std::fmt::Debug for ExtendedPubKey {
    // never print the key: it reveals every address of the wallet
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtendedPubKey")
            .field("kinds", &self.kinds)
            .field("depth", &self.depth)
            .finish_non_exhaustive()
    }
}

/// Why a key was rejected; the caller turns it into a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// Not Base58Check, wrong length or checksum.
    Malformed,
    /// A private key (xprv …): never accepted.
    Private,
    /// Testnet, multisig (Ypub/Zpub) or an unknown version.
    Unsupported,
    /// The key of one address chain (`m/84'/0'/0'/0`, depth 4): its change
    /// addresses are out of reach, so every payment would look like a full loss.
    ChainLevel,
}

const XPUB: [u8; 4] = [0x04, 0x88, 0xB2, 0x1E];
const YPUB: [u8; 4] = [0x04, 0x9D, 0x7C, 0xB2];
const ZPUB: [u8; 4] = [0x04, 0xB2, 0x47, 0x46];
const PRIVATE: [[u8; 4]; 3] = [
    [0x04, 0x88, 0xAD, 0xE4], // xprv
    [0x04, 0x9D, 0x78, 0x78], // yprv
    [0x04, 0xB2, 0x43, 0x0C], // zprv
];

impl ExtendedPubKey {
    /// Accepts the bare key or an output descriptor as wallets export it, e.g.
    /// `wpkh([d34db33f/84'/0'/0']xpub…/<0;1>/*)#checksum`; the descriptor's
    /// script type wins over the key's prefix.
    pub fn parse_input(text: &str) -> Result<Self, KeyError> {
        let mut t = text.trim();
        if let Some((body, _checksum)) = t.split_once('#') {
            t = body.trim();
        }
        let mut kind = None;
        for (prefix, k) in [
            ("sh(wpkh(", Some(ScriptKind::P2shP2wpkh)),
            ("wpkh(", Some(ScriptKind::P2wpkh)),
            ("pkh(", Some(ScriptKind::P2pkh)),
            // Taproot prefix; the escaped bracket keeps the i18n source scan from reading a `tr` call
            ("tr\u{28}", None),
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let Some(k) = k else {
                    return Err(KeyError::Unsupported); // Taproot is not derived yet
                };
                kind = Some(k);
                t = rest.trim_end_matches(')');
                break;
            }
        }
        if t.starts_with('[') {
            t = t
                .split_once(']')
                .map(|(_, r)| r)
                .ok_or(KeyError::Malformed)?;
        }
        let bare = t.split('/').next().unwrap_or(t).trim();
        let mut key = Self::parse(bare)?;
        if key.depth == 4 {
            return Err(KeyError::ChainLevel);
        }
        if let Some(k) = kind {
            key.kinds = vec![k];
        }
        Ok(key)
    }

    pub fn parse(text: &str) -> Result<Self, KeyError> {
        let raw = base58check_decode(text.trim()).ok_or(KeyError::Malformed)?;
        if raw.len() != 78 {
            return Err(KeyError::Malformed);
        }
        let version: [u8; 4] = raw[0..4].try_into().map_err(|_| KeyError::Malformed)?;
        if PRIVATE.contains(&version) || raw[45] == 0 {
            return Err(KeyError::Private);
        }
        let kinds = match version {
            // SegWit first: most xpub exports today are native SegWit accounts
            XPUB => vec![ScriptKind::P2wpkh, ScriptKind::P2pkh],
            YPUB => vec![ScriptKind::P2shP2wpkh],
            ZPUB => vec![ScriptKind::P2wpkh],
            _ => return Err(KeyError::Unsupported),
        };
        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&raw[13..45]);
        let mut key = [0u8; 33];
        key.copy_from_slice(&raw[45..78]);
        point(&key).ok_or(KeyError::Malformed)?;
        Ok(Self {
            chain_code,
            key,
            kinds,
            depth: raw[4],
        })
    }

    /// BIP32 public child derivation (non-hardened `index` only).
    pub fn child(&self, index: u32) -> Option<([u8; 32], [u8; 33])> {
        derive(&self.chain_code, &self.key, index)
    }

    /// Public key at `<account>/<chain>/<index>` (chain 0 = receive, 1 = change).
    pub fn public_key(&self, chain: u32, index: u32) -> Option<[u8; 33]> {
        let (cc, k) = self.child(chain)?;
        derive(&cc, &k, index).map(|(_, key)| key)
    }
}

fn point(key: &[u8; 33]) -> Option<AffinePoint> {
    let enc = Sec1Point::from_bytes(key).ok()?;
    Option::from(AffinePoint::from_sec1_point(&enc))
}

fn derive(chain_code: &[u8; 32], key: &[u8; 33], index: u32) -> Option<([u8; 32], [u8; 33])> {
    if index & 0x8000_0000 != 0 {
        return None; // hardened children need the private key
    }
    let mut mac = Hmac::<Sha512>::new_from_slice(chain_code).ok()?;
    mac.update(key);
    mac.update(&index.to_be_bytes());
    let i = mac.finalize().into_bytes();
    let mut il = k256::FieldBytes::default();
    il.copy_from_slice(&i[..32]);
    // IL ≥ n: this child does not exist (BIP32), the caller skips the index
    let tweak: Scalar = Option::from(Scalar::from_repr(il))?;
    let child = ProjectivePoint::GENERATOR * tweak + ProjectivePoint::from(point(key)?);
    let affine = child.to_affine();
    let enc = affine.to_sec1_point(true);
    let bytes: [u8; 33] = enc.as_bytes().try_into().ok()?; // identity point has no 33-byte form
    let mut cc = [0u8; 32];
    cc.copy_from_slice(&i[32..]);
    Some((cc, bytes))
}

pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let mut out = [0u8; 20];
    out.copy_from_slice(&Ripemd160::digest(sha));
    out
}

/// Mainnet address of `key` for `kind`.
pub fn address(kind: ScriptKind, key: &[u8; 33]) -> String {
    let h = hash160(key);
    match kind {
        ScriptKind::P2pkh => base58check_encode(0x00, &h),
        ScriptKind::P2shP2wpkh => {
            let mut redeem = vec![0x00, 0x14];
            redeem.extend_from_slice(&h);
            base58check_encode(0x05, &hash160(&redeem))
        }
        ScriptKind::P2wpkh => segwit_v0_address("bc", &h),
    }
}

// ------------------------------------------------------------------ Base58Check

const B58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn checksum(data: &[u8]) -> [u8; 4] {
    let twice = Sha256::digest(Sha256::digest(data));
    [twice[0], twice[1], twice[2], twice[3]]
}

pub fn base58check_encode(version: u8, payload: &[u8]) -> String {
    let mut data = vec![version];
    data.extend_from_slice(payload);
    let sum = checksum(&data);
    data.extend_from_slice(&sum);
    let zeros = data.iter().take_while(|&&b| b == 0).count();
    // repeated division by 58 on a big-endian byte number
    let mut digits: Vec<u8> = Vec::new();
    let mut num = data[zeros..].to_vec();
    while !num.is_empty() {
        let mut rem = 0u32;
        let mut next = Vec::with_capacity(num.len());
        for &b in &num {
            let acc = rem * 256 + b as u32;
            let q = acc / 58;
            rem = acc % 58;
            if !next.is_empty() || q != 0 {
                next.push(q as u8);
            }
        }
        digits.push(B58[rem as usize]);
        num = next;
    }
    let mut out = "1".repeat(zeros);
    out.extend(digits.iter().rev().map(|&c| c as char));
    out
}

pub fn base58check_decode(text: &str) -> Option<Vec<u8>> {
    if text.is_empty() || text.len() > 200 {
        return None;
    }
    let zeros = text.bytes().take_while(|&c| c == b'1').count();
    let mut num: Vec<u8> = Vec::new(); // big-endian
    for c in text.bytes().skip(zeros) {
        let mut carry = B58.iter().position(|&d| d == c)? as u32;
        for b in num.iter_mut().rev() {
            let acc = *b as u32 * 58 + carry;
            *b = (acc & 0xFF) as u8;
            carry = acc >> 8;
        }
        while carry > 0 {
            num.insert(0, (carry & 0xFF) as u8);
            carry >>= 8;
        }
    }
    let mut data = vec![0u8; zeros];
    data.extend(num);
    if data.len() < 5 {
        return None;
    }
    let (body, sum) = data.split_at(data.len() - 4);
    (checksum(body) == sum).then(|| body.to_vec())
}

// ------------------------------------------------------------------ Bech32 (BIP173)

const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn polymod(values: &[u8]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk = 1u32;
    for &v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ v as u32;
        for (i, g) in GEN.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn convert_bits(data: &[u8], from: u32, to: u32) -> Vec<u8> {
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut out = Vec::new();
    let max = (1u32 << to) - 1;
    for &v in data {
        acc = (acc << from) | v as u32;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & max) as u8);
        }
    }
    if bits > 0 {
        out.push(((acc << (to - bits)) & max) as u8);
    }
    out
}

/// Witness version 0 address (bech32, not bech32m).
pub fn segwit_v0_address(hrp: &str, program: &[u8]) -> String {
    let mut data = vec![0u8];
    data.extend(convert_bits(program, 8, 5));
    let mut values: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    values.push(0);
    values.extend(hrp.bytes().map(|b| b & 31));
    values.extend(&data);
    values.extend([0u8; 6]);
    let pm = polymod(&values) ^ 1;
    let mut out = format!("{hrp}1");
    out.extend(data.iter().map(|&d| CHARSET[d as usize] as char));
    out.extend((0..6).map(|i| CHARSET[((pm >> (5 * (5 - i))) & 31) as usize] as char));
    out
}
