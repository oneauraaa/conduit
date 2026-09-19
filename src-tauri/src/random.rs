/// Returns 128 bits from the operating system's cryptographic random source.
///
/// Keeping this tiny platform adapter in one place also avoids `getrandom`'s
/// `ProcessPrng` import on Windows. Conduit already supports Windows versions
/// through the documented BCrypt API used here.
#[cfg(not(target_os = "windows"))]
fn bytes() -> [u8; 16] {
    use std::io::Read;

    let mut bytes = [0u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        if file.read_exact(&mut bytes).is_ok() {
            return bytes;
        }
    }
    panic!("could not read /dev/urandom to generate a random identifier");
}

#[cfg(target_os = "windows")]
fn bytes() -> [u8; 16] {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom,
    };

    let mut bytes = [0u8; 16];
    // A null algorithm handle with USE_SYSTEM_PREFERRED_RNG is the documented
    // way to reach the system CSPRNG without opening a provider first.
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if status.is_ok() {
        return bytes;
    }
    panic!("could not reach the system CSPRNG to generate a random identifier: {status:?}");
}

pub(crate) fn token_hex() -> String {
    bytes().iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn uuid_v4() -> String {
    let mut value = bytes();
    value[6] = (value[6] & 0x0f) | 0x40;
    value[8] = (value[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_uuid_has_rfc_v4_bits() {
        let value = uuid::Uuid::parse_str(&uuid_v4()).unwrap();
        assert_eq!(value.get_version_num(), 4);
        assert_eq!(value.get_variant(), uuid::Variant::RFC4122);
    }
}
