use super::*;

fn hex(text: &str) -> Vec<u8> {
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap()).collect()
}

/// `count` bytes of a xorshift sequence from `seed`.
fn noise(seed: u32, count: usize) -> Vec<u8> {
    let mut state = seed | 1;
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

/// NIST SP 800-38A F.2.2, CBC-AES128.Decrypt; the vectors have no
/// padding, so a block of it, encrypted with the crate, follows them.
#[test]
fn decrypts_the_nist_vectors() {
    use aes::cipher::{BlockEncrypt, KeyInit};
    assume_from_environment();
    let key: [u8; 16] = hex("2b7e151628aed2a6abf7158809cf4f3c").try_into().unwrap();
    let iv: [u8; 16] = hex("000102030405060708090a0b0c0d0e0f").try_into().unwrap();
    let plain = hex(
        "6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51\
         30c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710",
    );
    let mut data = hex(
        "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2\
         73bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7",
    );
    let mut padding = aes::Block::from([16u8; 16]);
    padding.iter_mut().zip(&data[48..]).for_each(|(byte, previous)| *byte ^= previous);
    aes::Aes128::new(&key.into()).encrypt_block(&mut padding);
    data.extend_from_slice(&padding);
    decrypt(&key, &iv, &mut data).unwrap();
    assert_eq!(data, plain);
}

/// On 32-bit ARM, `MANA_AES_INSTRUCTIONS=1` has the tests use the AES
/// instructions: only where the CPU has them (the TV, QEMU).
fn assume_from_environment() {
    #[cfg(target_arch = "arm")]
    if std::env::var("MANA_AES_INSTRUCTIONS").is_ok_and(|v| v == "1") {
        // SAFETY: the one who runs the tests says the CPU has them.
        unsafe { assume_aes_instructions() };
        return;
    }
    eprintln!("provata solo la decifratura software");
}

#[test]
fn agrees_with_the_aes_crate_on_any_length_and_refuses_bad_padding() {
    assume_from_environment();
    for blocks in 1..=40 {
        let key: [u8; 16] = noise(blocks as u32, 16).try_into().unwrap();
        let iv: [u8; 16] = noise(blocks as u32 + 100, 16).try_into().unwrap();
        let data = noise(blocks as u32 + 200, blocks * BLOCK);
        let (mut ours, mut theirs) = (data.clone(), data.clone());
        let (ours_result, theirs_result) = (decrypt(&key, &iv, &mut ours), software(&key, &iv, &mut theirs));
        assert_eq!(ours_result.is_ok(), theirs_result.is_ok(), "{blocks} blocchi");
        if ours_result.is_ok() {
            assert_eq!(ours, theirs, "{blocks} blocchi");
        }
    }
    // Valid padding of every length, through encryption with the crate.
    for count in 1..=BLOCK {
        use aes::cipher::{BlockEncryptMut, KeyIvInit};
        let (key, iv) = ([7u8; 16], [9u8; 16]);
        let plain = noise(count as u32, 3 * BLOCK - count);
        let mut buffer = plain.clone();
        buffer.resize(3 * BLOCK, 0);
        let encryptor = cbc::Encryptor::<aes::Aes128>::new(&key.into(), &iv.into());
        let mut data = encryptor.encrypt_padded_mut::<Pkcs7>(&mut buffer, plain.len()).unwrap().to_vec();
        decrypt(&key, &iv, &mut data).unwrap();
        assert_eq!(data, plain, "{count} bytes of padding");
    }
    assert!(decrypt(&[0; 16], &[0; 16], &mut vec![0; 15]).is_err());
    assert!(decrypt(&[0; 16], &[0; 16], &mut Vec::new()).is_err());
}
