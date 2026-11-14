use super::super::keyfile::UserKey;
use super::*;
use openssl::rsa::Rsa;

fn bank_key(key: &PKey<Private>, user: &str, kind: u8) -> BankKey {
    BankKey {
        blz: "12345678".into(),
        user_id: user.into(),
        kind,
        number: 1,
        version: 1,
        key: PKey::public_key_from_der(&key.public_key_to_der().unwrap()).unwrap(),
    }
}

fn peer(own: PKey<Private>, remote: &PKey<Private>, user: &str, other: &str) -> Rah10 {
    Rah10::new(
        KeyFile {
            identities: vec![Identity {
                blz: "12345678".into(),
                user_id: user.into(),
                customer_id: user.into(),
                system_id: "1".into(),
                host: "example.invalid".into(),
                signature_id: 1,
                signing: UserKey {
                    number: 1,
                    version: 1,
                    key: own.clone(),
                },
                encryption: UserKey {
                    number: 1,
                    version: 1,
                    key: own,
                },
            }],
            bank_keys: vec![bank_key(remote, other, 0), bank_key(remote, other, 1)],
        },
        user,
        2,
        100,
    )
    .unwrap()
}

#[test]
fn encrypted_signed_exchange_rejects_tampering_and_wrong_key() {
    let client_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let bank_key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let client = peer(client_key.clone(), &bank_key, "client", "bank");
    let mut bank = peer(bank_key, &client_key, "bank", "client");
    let payload = vec![SegmentBuilder::new("HIRMG", 3, 2).raw("0010::OK").build()];
    let wire = bank.build(&payload, "1", "dialog", 1).unwrap();
    let result = client.unwrap(&wire, 1, "dialog").unwrap();
    assert_eq!(result[1].id(), "HIRMG");
    assert!(client.unwrap(&wire, 2, "dialog").is_err());
    assert!(client.unwrap(&wire, 1, "different-dialog").is_err());
    for len in 0..wire.len() {
        assert!(client.unwrap(&wire[..len], 1, "dialog").is_err());
    }
    let mut corrupted = wire.clone();
    let at = wire.windows(6).position(|b| b == b"HNVSD:").unwrap() + 25;
    corrupted[at] ^= 1;
    assert!(client.unwrap(&corrupted, 1, "dialog").is_err());
    let wrong = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let wrong_client = peer(client_key, &wrong, "client", "bank");
    assert!(wrong_client.unwrap(&wire, 1, "dialog").is_err());
    bank.next_signature = bank.signature_limit;
    assert!(bank.build(&payload, "1", "dialog", 2).is_err());
}

#[test]
fn signature_covers_exact_bytes_and_double_hash() {
    let private = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let public = PKey::public_key_from_der(&private.public_key_to_der().unwrap()).unwrap();
    let data = b"HNSHK:2:4+RAH:10'HIRMG:3:2+0010::OK'";
    let signature = sign(&private, data).unwrap();
    verify(&public, data, &signature).unwrap();
    assert!(verify(&public, b"changed", &signature).is_err());
    let mut single_hash = Verifier::new(MessageDigest::sha256(), &public).unwrap();
    single_hash.set_rsa_padding(Padding::PKCS1_PSS).unwrap();
    single_hash
        .set_rsa_pss_saltlen(RsaPssSaltlen::custom(32))
        .unwrap();
    single_hash.update(data).unwrap();
    assert!(!single_hash.verify(&signature).unwrap());
}

#[test]
fn segment_ranges_skip_binary_and_escapes() {
    let data = b"TEST:1:1+@3@a'b'TEST:2:1+foo?'bar'";
    let ranges = segment_ranges(data).unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(&data[ranges[0].0..ranges[0].1], b"TEST:1:1+@3@a'b'");
    for bad in [b"TEST+@99999@x'".as_slice(), b"TEST+?", b"TEST"] {
        assert!(segment_ranges(bad).is_err());
    }
}

#[test]
fn missing_bank_keys_require_setup() {
    let own = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let remote = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let client = peer(own, &remote, "client", "bank");
    let file = KeyFile {
        identities: vec![client.identity],
        bank_keys: vec![],
    };
    let error = Rah10::new(file, "client", 2, 100).err().unwrap();
    assert!(matches!(error, CoreError::InvalidInput { .. }));
    assert!(error.to_string().contains("Bankschlüssel"));
}
