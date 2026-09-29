use super::*;

use std::os::unix::fs::PermissionsExt as _;

fn workdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-sign-{}-{tag}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn hmac_matches_rfc4231_test_case_2() {
    // RFC 4231 TC2: key = "Jefe", data = "what do ya want for nothing?"
    let k = Key::from_secret(b"Jefe".to_vec()).unwrap();
    let got = k.sign_bytes(b"what do ya want for nothing?").unwrap();
    assert_eq!(
        got,
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn generate_load_sign_verify_and_tamper_detection() {
    let d = workdir("roundtrip");
    let key = d.join("k.hex");
    Key::generate(&key).unwrap();
    let mode = fs::metadata(&key).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "key file must be owner-only");
    assert!(
        Key::generate(&key).is_err(),
        "must refuse to overwrite a live key"
    );

    let k = Key::load(&key).unwrap();
    let art = d.join("evidence.json");
    fs::write(&art, b"{\"totals\":{\"requests\":7}}\n").unwrap();
    let sig = k.sign_file(&art).unwrap();
    assert_eq!(sig, d.join("evidence.json.sig"));
    k.verify_file(&art, &sig).unwrap();

    // One flipped byte in the artifact must be detected.
    let mut b = fs::read(&art).unwrap();
    b[2] = b'9';
    fs::write(&art, &b).unwrap();
    let e = k.verify_file(&art, &sig).unwrap_err();
    assert!(e.to_string().contains("FAILED"), "{e}");

    // ...and a different key must be rejected even for the intact file.
    let key2 = d.join("k2.hex");
    Key::generate(&key2).unwrap();
    fs::write(&art, b"{\"totals\":{\"requests\":7}}\n").unwrap();
    assert!(Key::load(&key2).unwrap().verify_file(&art, &sig).is_err());
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn key_files_are_strictly_validated() {
    let d = workdir("validate");
    let p = d.join("k.hex");

    fs::write(&p, b"nothexnothexnothexnothexnothexnothexnothex\n").unwrap();
    let e = format!("{:#}", Key::load(&p).unwrap_err()); // full anyhow chain
    assert!(e.contains("invalid hex digit"), "{e}");

    fs::write(&p, b"\n").unwrap();
    let e = Key::load(&p).unwrap_err().to_string();
    assert!(e.contains("empty"), "{e}");

    fs::write(&p, b"00112233\n").unwrap(); // 4 bytes: below policy floor
    let e = Key::load(&p).unwrap_err().to_string();
    assert!(e.contains("32 required"), "{e}");

    // Uppercase hex is accepted and means the same key.
    let lower = "ab".repeat(32);
    fs::write(&p, format!("{lower}\n")).unwrap();
    let a = Key::load(&p).unwrap();
    fs::write(&p, format!("{}\n", lower.to_uppercase())).unwrap();
    let b = Key::load(&p).unwrap();
    assert_eq!(a.secret, b.secret);
    let _ = fs::remove_dir_all(&d);
}
