//! RAH-10 message protection. No payment jobs are enabled by this module.
//! Protocol reference: HBCI4Java HBCIPassportRAH10, Sig and Crypt.
use openssl::{
    hash::{hash, MessageDigest},
    pkey::{PKey, Private, Public},
    rand::rand_bytes,
    rsa::Padding,
    sign::{RsaPssSaltlen, Signer, Verifier},
    symm::{Cipher, Crypter, Mode},
};
use zeroize::Zeroizing;

use super::{
    keyfile::{BankKey, Identity, KeyFile},
    syntax::{self, SegmentBuilder, Value},
};
use crate::error::{CoreError, CoreResult};

fn error() -> CoreError {
    CoreError::Protocol {
        message: "RAH-10-Nachricht unvollständig oder kryptografisch nicht verifiziert.".into(),
    }
}
fn crypto(_: openssl::error::ErrorStack) -> CoreError {
    error()
}

pub struct Rah10 {
    pub identity: Identity,
    bank_sign: BankKey,
    bank_encrypt: BankKey,
    next_signature: u64,
    signature_limit: u64,
}

fn aes(data: &[u8], key: &[u8], mode: Mode) -> CoreResult<Zeroizing<Vec<u8>>> {
    let mut cipher =
        Crypter::new(Cipher::aes_256_cbc(), mode, key, Some(&[0; 16])).map_err(crypto)?;
    cipher.pad(false);
    let mut out = Zeroizing::new(vec![0; data.len() + 16]);
    let used = cipher.update(data, &mut out).map_err(crypto)?;
    let end = cipher.finalize(&mut out[used..]).map_err(crypto)?;
    out.truncate(used + end);
    Ok(out)
}

fn sign(key: &PKey<Private>, data: &[u8]) -> CoreResult<Vec<u8>> {
    let digest = hash(MessageDigest::sha256(), data).map_err(crypto)?;
    let mut signer = Signer::new(MessageDigest::sha256(), key).map_err(crypto)?;
    signer.set_rsa_padding(Padding::PKCS1_PSS).map_err(crypto)?;
    signer
        .set_rsa_mgf1_md(MessageDigest::sha256())
        .map_err(crypto)?;
    signer
        .set_rsa_pss_saltlen(RsaPssSaltlen::custom(32))
        .map_err(crypto)?;
    signer.update(&digest).map_err(crypto)?;
    signer.sign_to_vec().map_err(crypto)
}

fn verify(key: &PKey<Public>, data: &[u8], signature: &[u8]) -> CoreResult<()> {
    let digest = hash(MessageDigest::sha256(), data).map_err(crypto)?;
    let mut verifier = Verifier::new(MessageDigest::sha256(), key).map_err(crypto)?;
    verifier
        .set_rsa_padding(Padding::PKCS1_PSS)
        .map_err(crypto)?;
    verifier
        .set_rsa_mgf1_md(MessageDigest::sha256())
        .map_err(crypto)?;
    verifier
        .set_rsa_pss_saltlen(RsaPssSaltlen::custom(32))
        .map_err(crypto)?;
    verifier.update(&digest).map_err(crypto)?;
    if !verifier.verify(signature).map_err(crypto)? {
        return Err(error());
    }
    Ok(())
}

impl Rah10 {
    pub fn new(mut file: KeyFile, login: &str, first: u64, limit: u64) -> CoreResult<Self> {
        let index = file
            .identities
            .iter()
            .position(|i| i.user_id == login)
            .ok_or_else(error)?;
        let identity = file.identities.remove(index);
        let take = |keys: &mut Vec<BankKey>, kind| -> CoreResult<BankKey> {
            let matches: Vec<_> = keys
                .iter()
                .enumerate()
                .filter(|(_, k)| k.blz == identity.blz && k.kind == kind)
                .map(|(i, _)| i)
                .collect();
            if matches.is_empty() {
                return Err(CoreError::InvalidInput {
                    message: "Öffentliche Bankschlüssel fehlen. Bitte im Einrichtungsdialog die Bankschlüssel abrufen und ihre Prüfwerte unabhängig mit der Commerzbank-Quelle vergleichen.".into(),
                });
            }
            if matches.len() != 1 {
                return Err(error());
            }
            Ok(keys.remove(matches[0]))
        };
        let bank_sign = take(&mut file.bank_keys, 0)?;
        let bank_encrypt = take(&mut file.bank_keys, 1)?;
        if first <= u64::from(identity.signature_id) || limit <= first {
            return Err(error());
        }
        Ok(Self {
            identity,
            bank_sign,
            bank_encrypt,
            next_signature: first,
            signature_limit: limit,
        })
    }

    pub fn build(
        &mut self,
        payload: &[Vec<u8>],
        system_id: &str,
        dialog: &str,
        message: u32,
    ) -> CoreResult<Vec<u8>> {
        if self.next_signature >= self.signature_limit {
            return Err(error());
        }
        let counter = self.next_signature;
        self.next_signature += 1; // consumed even if sending fails
        let now = chrono::Local::now();
        let timestamp = format!("1:{}:{}", now.format("%Y%m%d"), now.format("%H%M%S"));
        let reference = counter.to_string();
        let i = &self.identity;
        let keyname = |kind, number, version| {
            format!(
                "280:{}:{}:{kind}:{number}:{version}",
                i.blz,
                syntax::escape(&i.user_id)
            )
        };
        let mut signed = Zeroizing::new(
            SegmentBuilder::new("HNSHK", 2, 4)
                .raw("RAH:10")
                .raw("2")
                .raw(&reference)
                .raw("1")
                .raw("1")
                .raw(&format!("1::{}", syntax::escape(system_id)))
                .raw(&reference)
                .raw(&timestamp)
                .raw("1:6:1")
                .raw("6:10:19")
                .raw(&keyname("S", i.signing.number, i.signing.version))
                .build(),
        );
        for segment in payload {
            signed.extend_from_slice(segment);
        }
        let signature = sign(&i.signing.key, &signed)?;
        let tail = 3 + payload.len() as u32;
        signed.extend(
            SegmentBuilder::new("HNSHA", tail, 2)
                .raw(&reference)
                .binary(&signature)
                .build(),
        );
        // ISO 7816-4 padding (0x80 then zeroes), not PKCS#7.
        signed.push(0x80);
        while !signed.len().is_multiple_of(16) {
            signed.push(0);
        }
        let mut session_key = Zeroizing::new([0u8; 32]);
        rand_bytes(session_key.as_mut()).map_err(crypto)?;
        let bank_rsa = self.bank_encrypt.key.rsa().map_err(crypto)?;
        let mut padded = Zeroizing::new(vec![0; bank_rsa.size() as usize]);
        let start = padded.len() - 32;
        padded[start..].copy_from_slice(session_key.as_ref());
        let mut encrypted_key = vec![0; padded.len()];
        bank_rsa
            .public_encrypt(&padded, &mut encrypted_key, Padding::NONE)
            .map_err(crypto)?;
        let encrypted = aes(&signed, session_key.as_ref(), Mode::Encrypt)?;
        let mut head = SegmentBuilder::new("HNVSK", 998, 3)
            .raw("RAH:10")
            .raw("4")
            .raw("1")
            .raw(&format!("1::{}", syntax::escape(system_id)))
            .raw(&timestamp)
            .build();
        head.pop();
        head.extend_from_slice(format!("+2:2:14:@{}@", encrypted_key.len()).as_bytes());
        head.extend_from_slice(&encrypted_key);
        head.extend_from_slice(
            format!(
                ":6:1+280:{}:{}:V:{}:{}+0'",
                self.bank_encrypt.blz,
                syntax::escape(&self.bank_encrypt.user_id),
                self.bank_encrypt.number,
                self.bank_encrypt.version
            )
            .as_bytes(),
        );
        let data = SegmentBuilder::new("HNVSD", 999, 1)
            .binary(&encrypted)
            .build();
        let end = SegmentBuilder::new("HNHBS", tail + 1, 1)
            .raw(&message.to_string())
            .build();
        let assemble = |size| {
            let mut out = SegmentBuilder::new("HNHBK", 1, 3)
                .raw(&format!("{size:012}"))
                .raw("300")
                .de(&[dialog])
                .raw(&message.to_string())
                .build();
            out.extend_from_slice(&head);
            out.extend_from_slice(&data);
            out.extend_from_slice(&end);
            out
        };
        Ok(assemble(assemble(0).len()))
    }

    /// Reject unsigned responses and validate exact wire bytes before exposing payloads.
    pub fn unwrap(
        &self,
        raw: &[u8],
        expected_message: u32,
        dialog: &str,
    ) -> CoreResult<Vec<syntax::Segment>> {
        let outer = syntax::parse(raw)?;
        if outer.len() != 4
            || outer[0].id() != "HNHBK"
            || outer[1].id() != "HNVSK"
            || outer[2].id() != "HNVSD"
            || outer[3].id() != "HNHBS"
            || outer[0].get(1, 0).parse::<usize>().ok() != Some(raw.len())
            || outer[0].get(4, 0).parse::<u32>().ok() != Some(expected_message)
            || outer[3].get(1, 0).parse::<u32>().ok() != Some(expected_message)
            || (dialog != "0" && outer[0].get(3, 0) != dialog)
        {
            return Err(error());
        }
        let head = &outer[1];
        if head.get(1, 0) != "RAH"
            || head.get(1, 1) != "10"
            || head.get(2, 0) != "4"
            || head.get(6, 1) != "2"
            || head.get(6, 2) != "14"
            || head.get(8, 0) != "0"
        {
            return Err(error());
        }
        let enc_key = match head.element(6).get(3) {
            Some(Value::Binary(b)) => b,
            _ => return Err(error()),
        };
        let ciphertext = outer[2].binary(1).ok_or_else(error)?;
        if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(16) {
            return Err(error());
        }
        let rsa = self.identity.encryption.key.rsa().map_err(crypto)?;
        if enc_key.len() != rsa.size() as usize {
            return Err(error());
        }
        let mut decrypted = Zeroizing::new(vec![0; rsa.size() as usize]);
        rsa.private_decrypt(enc_key, &mut decrypted, Padding::NONE)
            .map_err(crypto)?;
        let split = decrypted.len() - 32;
        if decrypted[..split].iter().any(|&b| b != 0) {
            return Err(error());
        }
        let mut plain = aes(ciphertext, &decrypted[split..], Mode::Decrypt)?;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        if plain.pop() != Some(0x80) {
            return Err(error());
        }
        let ranges = segment_ranges(&plain)?;
        let inner = syntax::parse(&plain)?;
        if inner.len() < 3
            || inner[0].id() != "HNSHK"
            || inner.last().map(|s| s.id()) != Some("HNSHA")
        {
            return Err(error());
        }
        let h = &inner[0];
        let t = inner.last().ok_or_else(error)?;
        if h.get(1, 0) != "RAH"
            || h.get(1, 1) != "10"
            || h.get(4, 0) != "1"
            || h.get(9, 1) != "6"
            || h.get(10, 1) != "10"
            || h.get(10, 2) != "19"
            || h.get(3, 0) != t.get(1, 0)
            || h.get(11, 1) != self.bank_sign.blz
            || h.get(11, 2) != self.bank_sign.user_id
            || h.get(11, 4).parse::<u16>().ok() != Some(self.bank_sign.number)
            || h.get(11, 5).parse::<u16>().ok() != Some(self.bank_sign.version)
        {
            return Err(error());
        }
        let end = ranges.last().ok_or_else(error)?.0;
        verify(
            &self.bank_sign.key,
            &plain[..end],
            t.binary(2).ok_or_else(error)?,
        )?;
        let mut result = vec![outer[0].clone()];
        result.extend(inner.into_iter().skip(1).filter(|s| s.id() != "HNSHA"));
        Ok(result)
    }
}

// Exact segment ranges, skipping escaped bytes and length-delimited binary values.
fn segment_ranges(data: &[u8]) -> CoreResult<Vec<(usize, usize)>> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    while offset < data.len() {
        match data[offset] {
            b'?' => {
                offset += 2;
                if offset > data.len() {
                    return Err(error());
                }
            }
            b'@' => {
                let length_start = offset + 1;
                let end = data[length_start..]
                    .iter()
                    .position(|&b| b == b'@')
                    .ok_or_else(error)?
                    + length_start;
                let length: usize = std::str::from_utf8(&data[length_start..end])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(error)?;
                offset = (end + 1)
                    .checked_add(length)
                    .filter(|&n| n <= data.len())
                    .ok_or_else(error)?;
            }
            b'\'' => {
                offset += 1;
                result.push((start, offset));
                start = offset;
            }
            _ => offset += 1,
        }
    }
    if start != data.len() {
        return Err(error());
    }
    Ok(result)
}

/// Commerzbank RAH-10 endpoint; no host from an imported file is contacted.
pub async fn exchange(
    stream: &mut Option<tokio::net::TcpStream>,
    request: &[u8],
) -> CoreResult<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let operation = async {
        if stream.is_none() {
            *stream = Some(
                tokio::net::TcpStream::connect(("hbci.commerzbank.de", 3000))
                    .await
                    .map_err(|_| error())?,
            );
        }
        let socket = stream.as_mut().ok_or_else(error)?;
        socket.write_all(request).await.map_err(|_| error())?;
        let mut prefix = Vec::new();
        while prefix.iter().filter(|&&b| b == b'+').count() < 2 {
            if prefix.len() >= 64 {
                return Err(error());
            }
            prefix.push(socket.read_u8().await.map_err(|_| error())?);
        }
        if !prefix.starts_with(b"HNHBK:") {
            return Err(error());
        }
        let size = prefix
            .split(|&b| b == b'+')
            .nth(1)
            .and_then(|b| std::str::from_utf8(b).ok())
            .and_then(|s| s.parse::<usize>().ok())
            .ok_or_else(error)?;
        if size < prefix.len() || size > 32 * 1024 * 1024 {
            return Err(error());
        }
        let used = prefix.len();
        prefix.resize(size, 0);
        socket
            .read_exact(&mut prefix[used..])
            .await
            .map_err(|_| error())?;
        Ok(prefix)
    };
    tokio::time::timeout(std::time::Duration::from_secs(45), operation)
        .await
        .map_err(|_| error())?
}

#[cfg(test)]
mod tests;
