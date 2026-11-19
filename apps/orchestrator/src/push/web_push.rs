use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    aead, agreement, hkdf,
    rand::{SecureRandom, SystemRandom},
    signature::{self, EcdsaKeyPair, KeyPair},
};

const RECORD_SIZE: u32 = 4096;

#[derive(Debug)]
pub struct Unspecified;

impl From<ring::error::Unspecified> for Unspecified {
    fn from(_: ring::error::Unspecified) -> Self {
        Self
    }
}

impl From<ring::error::KeyRejected> for Unspecified {
    fn from(_: ring::error::KeyRejected) -> Self {
        Self
    }
}

pub struct Vapid(EcdsaKeyPair);

impl Vapid {
    pub fn generate() -> Result<Vec<u8>, Unspecified> {
        let document = EcdsaKeyPair::generate_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )?;
        Ok(document.as_ref().to_vec())
    }

    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Self, Unspecified> {
        Ok(Self(EcdsaKeyPair::from_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )?))
    }

    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.public_key().as_ref())
    }

    pub fn authorization(
        &self,
        audience: &str,
        subject: &str,
        expires: i64,
    ) -> Result<String, Unspecified> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": audience, "exp": expires, "sub": subject });
        let claims = URL_SAFE_NO_PAD.encode(claims.to_string());
        let signed = format!("{header}.{claims}");
        let signature = self.0.sign(&SystemRandom::new(), signed.as_bytes())?;
        Ok(format!(
            "vapid t={signed}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public_key()
        ))
    }
}

struct Length(usize);

impl hkdf::KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}

fn expand<const N: usize>(prk: &hkdf::Prk, info: &[&[u8]]) -> Result<[u8; N], Unspecified> {
    let mut out = [0; N];
    prk.expand(info, Length(N))?.fill(&mut out)?;
    Ok(out)
}

pub fn encrypt(receiver: &[u8], auth: &[u8], message: &[u8]) -> Result<Vec<u8>, Unspecified> {
    let random = SystemRandom::new();
    let ephemeral = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &random)?;
    let sender = ephemeral.compute_public_key()?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, receiver);
    let secret = agreement::agree_ephemeral(ephemeral, &peer, |shared| {
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(shared);
        expand::<32>(&prk, &[b"WebPush: info\0", receiver, sender.as_ref()])
    })??;
    let mut salt = [0; 16];
    random.fill(&mut salt)?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &salt).extract(&secret);
    let key = expand::<16>(&prk, &[b"Content-Encoding: aes128gcm\0"])?;
    let nonce = expand::<12>(&prk, &[b"Content-Encoding: nonce\0"])?;
    let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &key)?);
    let mut record = [message, &[2]].concat();
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::empty(),
        &mut record,
    )?;
    let sender = sender.as_ref();
    let id_length = u8::try_from(sender.len()).map_err(|_| Unspecified)?;
    Ok([
        &salt[..],
        &RECORD_SIZE.to_be_bytes(),
        &[id_length],
        sender,
        &record,
    ]
    .concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decrypt(
        receiver: agreement::EphemeralPrivateKey,
        public: &[u8],
        auth: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let (salt, rest) = body.split_at(16);
        assert_eq!(&rest[..4], &RECORD_SIZE.to_be_bytes());
        let (sender, record) = rest[5..].split_at(usize::from(rest[4]));
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, sender);
        let secret = agreement::agree_ephemeral(receiver, &peer, |shared| {
            let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, auth).extract(shared);
            expand::<32>(&prk, &[b"WebPush: info\0", public, sender]).unwrap()
        })
        .unwrap();
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, salt).extract(&secret);
        let key = expand::<16>(&prk, &[b"Content-Encoding: aes128gcm\0"]).unwrap();
        let nonce = expand::<12>(&prk, &[b"Content-Encoding: nonce\0"]).unwrap();
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &key).unwrap());
        let mut record = record.to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::empty(),
                &mut record,
            )
            .unwrap();
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }

    #[test]
    fn a_browser_reads_what_was_encrypted_for_it() {
        let random = SystemRandom::new();
        let receiver =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &random).unwrap();
        let public = receiver.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7; 16];
        let body = encrypt(&public, &auth, b"{\"title\":\"Forge\"}").unwrap();
        assert_eq!(
            decrypt(receiver, &public, &auth, &body),
            b"{\"title\":\"Forge\"}"
        );
    }

    #[test]
    fn the_vapid_token_verifies_against_the_announced_key() {
        let vapid = Vapid::from_pkcs8(&Vapid::generate().unwrap()).unwrap();
        let header = vapid
            .authorization("https://fcm.googleapis.com", "https://cubebot.lan", 1)
            .unwrap();
        let token = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        let (signed, signature) = token.rsplit_once('.').unwrap();
        let key = URL_SAFE_NO_PAD.decode(vapid.public_key()).unwrap();
        signature::UnparsedPublicKey::new(&signature::ECDSA_P256_SHA256_FIXED, key)
            .verify(
                signed.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .unwrap();
        let claims = URL_SAFE_NO_PAD
            .decode(signed.split('.').nth(1).unwrap())
            .unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&claims).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
    }
}
