use chacha20poly1305::{aead::{stream, KeyInit, OsRng}, {aead::rand_core::RngCore}, XChaCha20Poly1305};
use std::{fs, fs::File, io::{Read, Write}};
use std::fs::OpenOptions;
use argon2::Variant;
use chacha20poly1305::aead::Aead;
use zeroize::Zeroize;
use crate::{archiver, CryptoError};
use crate::common::{constant_time_compare_256_bit, get_file_stem_to_string, normalize_paths, sha3_32_hash};
use crate::CryptoError::{ChaCha20Poly1305Error, Message};
use crate::reed_solomon::{sr_encode_with_double_parity, sr_reconstruct_with_double_parity};


#[cfg(test)]
mod tests {
    use std::fs;
    use crate::CryptoError;
    // use zeroize::Zeroize;
    use crate::symmetric::{decrypt_file, encrypt_file};

    const SRC_FILE_PATH: &str = "src/test_files/test-file.txt";
    const SRC_DIR_PATH: &str = "src/test_files/test-folder";
    const ENCRYPTED_FILE_PATH: &str = "src/dest/test-file.fcs";
    const ENCRYPTED_DIR_PATH: &str = "src/dest/test-folder.fcs";
    const ENCRYPTED_LARGE_FILE_PATH: &str = "src/dest/test-archive.fcls";
    const DEST_DIR_PATH: &str = "src/dest/";
    const PASSPHRASE: &str = "strong_passphrase";

    #[test]
    fn encrypt_file_test() -> Result<(), CryptoError> {
        fs::create_dir_all("src/dest")?;
        // let mut passphrase = rpassword::prompt_password("passphrase:")?;
        let mut passphrase = PASSPHRASE.to_string();
        encrypt_file(SRC_FILE_PATH, DEST_DIR_PATH, &mut passphrase, false)?;

        // passphrase.zeroize();

        Ok(())
    }

    #[test]
    fn decrypt_file_test() -> Result<(), CryptoError> {
        // let mut password = rpassword::prompt_password("password:")?;
        let mut passphrase = PASSPHRASE.to_string();
        decrypt_file(ENCRYPTED_FILE_PATH, DEST_DIR_PATH, &mut passphrase)?;

        // password.zeroize();

        Ok(())
    }

    #[test]
    fn encrypt_dir_test() -> Result<(), CryptoError> {
        fs::create_dir_all("src/dest")?;
        // let mut passphrase = rpassword::prompt_password("passphrase:")?;
        let mut passphrase = PASSPHRASE.to_string();
        encrypt_file(SRC_DIR_PATH, DEST_DIR_PATH, &mut passphrase, false)?;

        // passphrase.zeroize();

        Ok(())
    }

    #[test]
    fn decrypt_dir_test() -> Result<(), CryptoError> {
        // let mut password = rpassword::prompt_password("password:")?;
        let mut passphrase = PASSPHRASE.to_string();
        decrypt_file(ENCRYPTED_DIR_PATH, DEST_DIR_PATH, &mut passphrase)?;

        // password.zeroize();

        Ok(())
    }

    #[test]
    fn encrypt_large_file_test() -> Result<(), CryptoError> {
        fs::create_dir_all("src/dest")?;
        // let mut passphrase = rpassword::prompt_password("passphrase:")?;
        let mut passphrase = PASSPHRASE.to_string();
        encrypt_file(SRC_FILE_PATH, DEST_DIR_PATH, &mut passphrase, true)?;

        // passphrase.zeroize();

        Ok(())
    }

    #[test]
    fn decrypt_large_file_test() -> Result<(), CryptoError> {
        // let mut password = rpassword::prompt_password("password:")?;
        let mut passphrase = "strong_passphrase".to_string();
        decrypt_file(ENCRYPTED_LARGE_FILE_PATH, DEST_DIR_PATH, &mut passphrase)?;

        // password.zeroize();

        Ok(())
    }
}

// Encrypt file with XChaCha20Poly1305 algorithm
pub fn encrypt_file(input_path: &str, output_dir: &str, passphrase: &mut str, large: bool) -> Result<(), CryptoError> {
    let (input_path_norm, output_dir_norm) = normalize_paths(input_path, output_dir);
    let tmp_dir_path = &format!("{}zp_tmp/", output_dir_norm);
    fs::create_dir_all(tmp_dir_path)?;
    let file_stem = &archiver::archive(&input_path_norm, tmp_dir_path)?;
    let file_name_zipped = &format!("{}{}.zip", tmp_dir_path, file_stem);
    println!("\nencrypting {} ...", file_name_zipped);

    let argon2_config = argon2_config();
    let mut salt_32 = [0u8; 32];
    OsRng.fill_bytes(&mut salt_32);

    let mut key = argon2::hash_raw(passphrase.as_bytes(), &salt_32, &argon2_config)?;
    let cipher = XChaCha20Poly1305::new(key[..32].as_ref().into());

    // Hash the encryption key for comparison when decrypting
    let key_hash_ref: [u8; 32] = sha3_32_hash(&key)?;

    let encr_ext = if !large { "fcs" } else { "fcls" };

    let mut file_path_encrypted = OpenOptions::new()
        .write(true)
        .append(true)
        .create_new(true)
        .open(format!("{}{}.{}", &output_dir_norm, file_stem, encr_ext))?;


    // Encode with reed-solomon and serialize
    let salt_32_enc: Vec<Vec<u8>> = sr_encode_with_double_parity(&salt_32)?;
    let key_hash_ref_enc: Vec<Vec<u8>> = sr_encode_with_double_parity(&key_hash_ref)?;
    let salt_32_enc_ser: Vec<u8> = bincode::serialize(&salt_32_enc)?;
    let key_hash_ref_enc_ser: Vec<u8> = bincode::serialize(&key_hash_ref_enc)?;

    if !large {
        let mut nonce_24 = [0u8; 24];
        OsRng.fill_bytes(&mut nonce_24);

        let nonce_24_enc: Vec<Vec<u8>> = sr_encode_with_double_parity(&nonce_24)?;
        let nonce_24_enc_ser: Vec<u8> = bincode::serialize(&nonce_24_enc)?;
        let file_bytes_len = 0;

        // HEADER info for decrypting
        let header: [usize; 4] = [salt_32_enc_ser.len(), nonce_24_enc_ser.len(), key_hash_ref_enc_ser.len(), file_bytes_len];
        let header_ser: Vec<u8> = bincode::serialize(&header).unwrap();

        let source_file = fs::read(file_name_zipped)?;
        let ciphertext = cipher.encrypt(nonce_24.as_ref().into(), &*source_file)?;

        file_path_encrypted.write_all(&header_ser)?;
        file_path_encrypted.write_all(&salt_32_enc_ser)?;
        file_path_encrypted.write_all(&nonce_24_enc_ser)?;
        file_path_encrypted.write_all(&key_hash_ref_enc_ser)?;
        file_path_encrypted.write_all(&ciphertext)?;
    } else {
        let mut nonce_19 = [0u8; 19];
        OsRng.fill_bytes(&mut nonce_19);

        let mut stream_encryptor = stream::EncryptorBE32::from_aead(cipher, nonce_19.as_ref().into());

        // XChaCha20-Poly1305 is an AEAD cipher and appends a 16 bytes authentication tag to each encrypted message, so the buffer becomes 516 bits
        const BUFFER_LEN: usize = 500;
        let mut buffer = [0u8; BUFFER_LEN];

        file_path_encrypted.write_all(&salt_32)?;
        file_path_encrypted.write_all(&nonce_19)?;
        file_path_encrypted.write_all(&key_hash_ref)?;

        let mut source_file = File::open(file_name_zipped)?;
        loop {
            let read_count = source_file.read(&mut buffer)?;

            if read_count == BUFFER_LEN {
                let ciphertext = stream_encryptor
                    .encrypt_next(buffer.as_slice())
                    .map_err(ChaCha20Poly1305Error)?;
                file_path_encrypted.write_all(&ciphertext)?;
            } else {
                let ciphertext = stream_encryptor
                    .encrypt_last(&buffer[..read_count])
                    .map_err(ChaCha20Poly1305Error)?;
                file_path_encrypted.write_all(&ciphertext)?;
                break;
            }
        }
    }

    fs::remove_dir_all(tmp_dir_path)?;

    let file_name_encrypted = &format!("{}{}.{}", output_dir_norm, file_stem, encr_ext);
    println!("\nencrypted to {}", file_name_encrypted);

    key.zeroize();
    passphrase.zeroize();

    Ok(())
}

pub fn decrypt_file(input_path: &str, output_dir: &str, passphrase: &mut str) -> Result<(), CryptoError> {
    let (input_path_norm, output_dir_norm) = normalize_paths(input_path, output_dir);

    if input_path_norm.ends_with(".fcs") {
        decrypt_normal_file(&input_path_norm, &output_dir_norm, passphrase)?;
    } else if input_path_norm.ends_with(".fcls") {
        decrypt_large_file(&input_path_norm, &output_dir_norm, passphrase)?;
    }

    println!("\ndecrypted to {}", output_dir_norm);

    Ok(())
}

// Decrypt file with XChaCha20Poly1305 algorithm
fn decrypt_normal_file(input_path: &str, output_dir: &str, passphrase: &mut str) -> Result<(), CryptoError> {
    if input_path.ends_with(".fcs") {
        println!("decrypting {} ...\n", input_path);
        let encrypted_file: Vec<u8> = fs::read(input_path)?;

        // Split salt, nonce, key hash and the encrypted file
        // Deserialize and reconstruct with reed-solomon
        let (header_bytes, rem_data) = encrypted_file.split_at(32);
        let header: [usize; 4] = bincode::deserialize(header_bytes)?;
        let (salt_enc_bytes, rem_data) = rem_data.split_at(header[0]);
        let (nonce_enc_bytes, rem_data) = rem_data.split_at(header[1]);
        let (key_hash_ref_enc_bytes, ciphertext) = rem_data.split_at(header[2]);

        let salt_enc: Vec<Vec<u8>> = bincode::deserialize(salt_enc_bytes)?;
        let nonce_enc: Vec<Vec<u8>> = bincode::deserialize(nonce_enc_bytes)?;
        let key_hash_ref_enc: Vec<Vec<u8>> = bincode::deserialize(key_hash_ref_enc_bytes)?;

        let salt = sr_reconstruct_with_double_parity(salt_enc, 32)?;
        let nonce_24 = sr_reconstruct_with_double_parity(nonce_enc, 24)?;
        let key_hash_ref = sr_reconstruct_with_double_parity(key_hash_ref_enc, 32)?;

        let argon2_config = argon2_config();
        let mut key = argon2::hash_raw(passphrase.as_bytes(), &salt[0..32], &argon2_config)?;

        // Hash the encryption key for comparison and compare it in constant time with the ref key hash
        let key_hash: [u8; 32] = sha3_32_hash(&key)?;
        let key_correct = constant_time_compare_256_bit(&key_hash, key_hash_ref[0..32].try_into()?);

        if key_correct {
            let tmp_dir_path = &format!("{}zp_tmp/", output_dir);
            fs::create_dir_all(tmp_dir_path)?;
            let cipher = XChaCha20Poly1305::new(key[..32].as_ref().into());
            let plaintext: Vec<u8> = cipher.decrypt(nonce_24[0..24].as_ref().into(), ciphertext.as_ref())?;
            let file_stem_decrypted = &get_file_stem_to_string(input_path)?;
            let decrypted_file_path: String = format!("{}{}.zip", tmp_dir_path, file_stem_decrypted);

            File::create(&decrypted_file_path)?;
            fs::write(&decrypted_file_path, plaintext)?;
            archiver::unarchive(&decrypted_file_path, output_dir)?;

            key.zeroize();
            passphrase.zeroize();

            fs::remove_dir_all(tmp_dir_path)?;
        } else {
            return Err(Message("The provided password is incorrect!".to_string()));
        }
    } else {
        return Err(Message("This file should have '.fcs' extension!".to_string()));
    }

    Ok(())
}

// Decrypt large file, that doesn't fit in RAM, with XChaCha20Poly1305 algorithm. This is much slower
fn decrypt_large_file(input_path: &str, output_dir: &str, passphrase: &mut str) -> Result<(), CryptoError> {
    if input_path.ends_with(".fcls") {
        println!("decrypting {} ...\n", input_path);

        let mut salt = [0u8; 32];
        let mut nonce_19 = [0u8; 19];
        let mut key_hash_ref = [0u8; 32];
        let mut encrypted_file = File::open(input_path)?;
        let mut read_count = encrypted_file.read(&mut salt)?;

        if read_count != salt.len() {
            return Err(Message("Error reading salt!".to_string()));
        }

        read_count = encrypted_file.read(&mut nonce_19)?;
        if read_count != nonce_19.len() {
            return Err(Message("Error reading nonce!".to_string()));
        }

        read_count = encrypted_file.read(&mut key_hash_ref)?;
        if read_count != key_hash_ref.len() {
            return Err(Message("Error reading key_hash_ref!".to_string()));
        }

        let argon2_config = argon2_config();
        let mut key = argon2::hash_raw(passphrase.as_bytes(), &salt, &argon2_config)?;

        // Hash the encryption key for comparison and compare it in constant time with the ref key hash
        let key_hash: [u8; 32] = sha3_32_hash(&key)?;
        let key_correct = constant_time_compare_256_bit(&key_hash, key_hash_ref[..32].try_into()?);

        if key_correct {
            let tmp_dir_path = &format!("{}zp_tmp/", output_dir);
            fs::create_dir_all(tmp_dir_path)?;
            let cipher = XChaCha20Poly1305::new(key[..32].as_ref().into());
            let mut stream_decryptor = stream::DecryptorBE32::from_aead(cipher, nonce_19.as_ref().into());
            let file_stem_decrypted = &get_file_stem_to_string(input_path)?;
            let decrypted_file_path = format!("{}{}.zip", tmp_dir_path, file_stem_decrypted);
            let mut decrypted_file = OpenOptions::new()
                .write(true)
                .append(true)
                .create_new(true)
                .open(&decrypted_file_path)?;

            // 500 bytes for the encrypted piece of data, and 16 bytes for the authentication tag, which was added on encryption
            const BUFFER_LEN: usize = 500 + 16;
            let mut buffer = [0u8; BUFFER_LEN];

            loop {
                let read_count = encrypted_file.read(&mut buffer)?;

                if read_count == BUFFER_LEN {
                    let plaintext = stream_decryptor
                        .decrypt_next(buffer.as_slice())
                        .map_err(ChaCha20Poly1305Error)?;
                    decrypted_file.write_all(&plaintext)?;
                } else if read_count == 0 {
                    break;
                } else {
                    let plaintext = stream_decryptor
                        .decrypt_last(&buffer[..read_count])
                        .map_err(ChaCha20Poly1305Error)?;
                    decrypted_file.write_all(&plaintext)?;
                    break;
                }
            }

            archiver::unarchive(&decrypted_file_path, output_dir)?;

            key.zeroize();
            passphrase.zeroize();

            fs::remove_dir_all(tmp_dir_path)?;
        } else {
            return Err(Message("The provided password is incorrect!".to_string()));
        }
    } else {
        return Err(Message("This file should have '.fcls' extension!".to_string()));
    }

    Ok(())
}

fn argon2_config<'a>() -> argon2::Config<'a> {
    argon2::Config {
        variant: Variant::Argon2id,
        hash_length: 32,
        lanes: 8,
        mem_cost: 1024,
        time_cost: 8,
        ..Default::default()
    }
}
