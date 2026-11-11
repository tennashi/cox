//! Telling the human's phones of the chats that need them while they are
//! away from the Mac, as the Mac's notifications do while they are at it:
//! with Web Push (RFC 8030), which the phone's browser, once its page
//! subscribed, hears from its push service even with the page closed.
//!
//! What coxd pushes is encrypted for the phone alone (RFC 8291), with the
//! keys the page subscribed with, so the push service carries it unread;
//! and signed with a key of coxd's own (VAPID, RFC 8292), kept in
//! `state/push-key`, which the page subscribed with too, so that only
//! coxd pushes to it. Where to push to each phone is kept in
//! `state/pushes.json`, and only to the push services of the browsers,
//! since the page names it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Context as _;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{self, ECDH_P256, EphemeralPrivateKey, UnparsedPublicKey};
use ring::hmac::{self, HMAC_SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};
use serde::{Deserialize, Serialize};

use crate::storage::{self, Kept};

/// How long a push service keeps what coxd pushes for a phone it cannot
/// reach, as one switched off: a chat that waited a day is in the list.
const KEPT_FOR: Duration = Duration::from_hours(24);

/// How long the push service may take coxd's signature for coxd's: at
/// most a day, as the RFC has it.
const SIGNED_FOR: Duration = Duration::from_hours(12);

/// Who pushes, as the push services ask to be told, to reach whoever
/// runs it should it misbehave.
const PUSHED_BY: &str = "https://github.com/tennashi/cox";

/// How long a push may take.
const SEND_TIME: Duration = Duration::from_secs(30);

/// How much of what the chat said a push tells, in characters: the push
/// services carry 4 KB at most.
const TOLD: usize = 300;

/// The hosts of the push services of the browsers, the only ones coxd
/// pushes to: Chrome's, Firefox's, Safari's and Edge's.
const SERVICES: &[&str] = &[
    "fcm.googleapis.com",
    "push.services.mozilla.com",
    "push.apple.com",
    "notify.windows.com",
];

/// What a phone is told of a chat.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Notice {
    /// The number the chat goes by this run, for the page to show it.
    pub key: u64,
    /// The chat's name.
    pub title: String,
    pub body: String,
}

/// Where coxd pushes to a phone, as the page's `PushSubscription` says it
/// in JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    pub endpoint: String,
    pub keys: Keys,
}

/// The phone's keys, in URL-safe base64: its public key on P-256, and
/// the secret it shares with coxd.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    pub p256dh: String,
    pub auth: String,
}

/// A push to a phone, ready to go.
#[derive(Debug)]
pub struct Push {
    pub endpoint: String,
    /// `Authorization`, coxd's signature and key.
    pub authorization: String,
    /// What is pushed, encrypted for the phone.
    pub body: Vec<u8>,
}

/// How a push went.
#[derive(Debug, PartialEq, Eq)]
pub enum Pushed {
    Done,
    /// The phone does not take pushes there any more, as when the page
    /// unsubscribed, or its browser forgot it.
    Gone,
    Failed(String),
}

/// coxd's key, and where it pushes to the phones.
pub struct Pushes {
    key: EcdsaKeyPair,
    file: PathBuf,
    subscriptions: Vec<Subscription>,
    random: SystemRandom,
}

impl Pushes {
    /// coxd's key, made and kept the first time, and the phones it pushes
    /// to, as Cox keeps them.
    ///
    /// # Errors
    ///
    /// When there is no place to keep them, or the key cannot be read or
    /// made.
    pub fn open() -> anyhow::Result<Self> {
        let key = storage::path(Kept::PushKey).context("there is no place for the push key")?;
        let file = storage::path(Kept::Pushes).context("there is no place for the pushes")?;
        Self::open_at(&key, file)
    }

    pub(crate) fn open_at(key: &Path, file: PathBuf) -> anyhow::Result<Self> {
        let random = SystemRandom::new();
        let pkcs8 = match std::fs::read(key) {
            Ok(pkcs8) => pkcs8,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random)
                    .map_err(|_| anyhow::anyhow!("cannot make a push key"))?;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
                std::io::Write::write_all(&mut options.open(key)?, pkcs8.as_ref())
                    .with_context(|| format!("cannot keep {}", key.display()))?;
                pkcs8.as_ref().to_vec()
            }
            Err(error) => {
                return Err(error).with_context(|| format!("cannot read {}", key.display()));
            }
        };
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &random)
            .map_err(|_| anyhow::anyhow!("{} holds no push key", key.display()))?;
        let subscriptions = match std::fs::read(&file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .inspect_err(|error| tracing::error!("{} cannot be read: {error}", file.display()))
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        Ok(Self {
            key,
            file,
            subscriptions,
            random,
        })
    }

    /// coxd's public key, as the page subscribes with it: the point,
    /// uncompressed, in URL-safe base64.
    #[must_use]
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.public_key().as_ref())
    }

    /// Pushes to the phone `subscription` names from now on.
    ///
    /// # Errors
    ///
    /// When it names no push service of a browser, or keys of no phone.
    pub fn subscribe(&mut self, subscription: Subscription) -> Result<(), String> {
        of_a_browser(&subscription.endpoint)?;
        public_point(&subscription.keys.p256dh)?;
        shared_secret(&subscription.keys.auth)?;
        self.subscriptions
            .retain(|kept| kept.endpoint != subscription.endpoint);
        self.subscriptions.push(subscription);
        self.keep();
        Ok(())
    }

    /// Pushes to `endpoint` no more.
    pub fn unsubscribe(&mut self, endpoint: &str) {
        let before = self.subscriptions.len();
        self.subscriptions.retain(|kept| kept.endpoint != endpoint);
        if self.subscriptions.len() != before {
            self.keep();
        }
    }

    fn keep(&self) {
        let json = serde_json::to_vec_pretty(&self.subscriptions).unwrap_or_default();
        if let Err(error) = std::fs::write(&self.file, json) {
            tracing::error!("{} cannot be kept: {error}", self.file.display());
        }
    }

    /// What to push to each phone to tell it `notice`.
    #[must_use]
    pub fn pushes(&self, notice: &Notice) -> Vec<Push> {
        let mut notice = notice.clone();
        if notice.body.chars().count() > TOLD {
            notice.body = notice.body.chars().take(TOLD - 1).collect::<String>() + "…";
        }
        let told = serde_json::to_vec(&notice).unwrap_or_default();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        self.subscriptions
            .iter()
            .filter_map(|subscription| {
                let push = self.push(subscription, &told, now);
                if let Err(why) = &push {
                    let service = service(&subscription.endpoint);
                    tracing::error!(service, "cannot push: {why}");
                }
                push.ok()
            })
            .collect()
    }

    fn push(
        &self,
        subscription: &Subscription,
        told: &[u8],
        now: Duration,
    ) -> Result<Push, String> {
        let point = public_point(&subscription.keys.p256dh)?;
        let secret = shared_secret(&subscription.keys.auth)?;
        let body = encrypt(&point, &secret, told, &self.random)
            .map_err(|()| "cannot encrypt it".to_owned())?;
        let audience = url::Url::parse(&subscription.endpoint)
            .map_err(|error| error.to_string())?
            .origin()
            .ascii_serialization();
        let token = self.signed(&audience, now + SIGNED_FOR)?;
        Ok(Push {
            endpoint: subscription.endpoint.clone(),
            authorization: format!("vapid t={token}, k={}", self.public_key()),
            body,
        })
    }

    /// A JSON Web Token, signed with coxd's key, that says to the push
    /// service of `audience` that coxd pushes, until `until`.
    fn signed(&self, audience: &str, until: Duration) -> Result<String, String> {
        let head = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({
            "aud": audience,
            "exp": until.as_secs(),
            "sub": PUSHED_BY,
        });
        let claims = URL_SAFE_NO_PAD.encode(claims.to_string());
        let signed = format!("{head}.{claims}");
        let signature = self
            .key
            .sign(&self.random, signed.as_bytes())
            .map_err(|_| "cannot sign it".to_owned())?;
        Ok(format!(
            "{signed}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }

    /// Whether coxd pushes to any phone.
    #[must_use]
    pub fn any(&self) -> bool {
        !self.subscriptions.is_empty()
    }
}

/// Sends `push` to the phone's push service, waiting for it.
#[must_use]
pub fn send(push: &Push) -> Pushed {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_global(Some(SEND_TIME))
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let sent = agent
        .post(&push.endpoint)
        .header("TTL", KEPT_FOR.as_secs().to_string())
        .header("Urgency", "high")
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("Authorization", &push.authorization)
        .send(&push.body[..]);
    match sent {
        Ok(mut answer) => match answer.status().as_u16() {
            200..=299 => Pushed::Done,
            404 | 410 => Pushed::Gone,
            status => {
                let why = answer
                    .body_mut()
                    .with_config()
                    .limit(4096)
                    .read_to_string()
                    .unwrap_or_default();
                Pushed::Failed(format!("{status} {}", why.trim()))
            }
        },
        Err(error) => Pushed::Failed(error.to_string()),
    }
}

/// The push service of `endpoint`, by its host, to name it by in the log:
/// the endpoint itself is as good as a key to push to the phone.
#[must_use]
pub fn service(endpoint: &str) -> String {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Whether `endpoint` is a push service of a browser, over HTTPS.
fn of_a_browser(endpoint: &str) -> Result<(), String> {
    let url =
        url::Url::parse(endpoint).map_err(|error| format!("{endpoint} is no URL: {error}"))?;
    let host = url.host_str().unwrap_or_default();
    let known = SERVICES
        .iter()
        .any(|service| host == *service || host.ends_with(&format!(".{service}")));
    if url.scheme() == "https" && known && url.port().is_none() {
        Ok(())
    } else {
        Err(format!("{endpoint} is no push service of a browser"))
    }
}

/// `text` in URL-safe base64, padded or not, or in the standard alphabet.
fn decoded(text: &str) -> Result<Vec<u8>, String> {
    let text: String = text
        .trim_end_matches('=')
        .chars()
        .map(|char| match char {
            '+' => '-',
            '/' => '_',
            char => char,
        })
        .collect();
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|error| format!("not base64: {error}"))
}

/// The phone's public point on P-256, uncompressed.
fn public_point(text: &str) -> Result<Vec<u8>, String> {
    let point = decoded(text)?;
    if point.len() == 65 && point[0] == 4 {
        Ok(point)
    } else {
        Err("p256dh is no point on P-256".to_owned())
    }
}

/// The secret the phone shares with coxd.
fn shared_secret(text: &str) -> Result<Vec<u8>, String> {
    let secret = decoded(text)?;
    if secret.len() == 16 {
        Ok(secret)
    } else {
        Err("auth is no secret of 16 bytes".to_owned())
    }
}

/// HMAC-SHA-256 of `parts` with `key`.
fn mac(key: &[u8], parts: &[&[u8]]) -> hmac::Tag {
    let mut context = hmac::Context::with_key(&hmac::Key::new(HMAC_SHA256, key));
    for part in parts {
        context.update(part);
    }
    context.sign()
}

/// How long a record is, at most: what is pushed is one.
const RECORD: u32 = 4096;

/// `told` encrypted for the phone of the public `point` and the shared
/// `secret`, as RFC 8291 has it: one record of `aes128gcm` (RFC 8188),
/// whose header names the key coxd made for it alone.
fn encrypt(
    point: &[u8],
    secret: &[u8],
    told: &[u8],
    random: &dyn SecureRandom,
) -> Result<Vec<u8>, ()> {
    let ours = EphemeralPrivateKey::generate(&ECDH_P256, random).map_err(|_| ())?;
    let our_point = ours.compute_public_key().map_err(|_| ())?;
    let our_point = our_point.as_ref().to_vec();
    let shared = agreement::agree_ephemeral(
        ours,
        &UnparsedPublicKey::new(&ECDH_P256, point),
        <[u8]>::to_vec,
    )
    .map_err(|_| ())?;
    let mut salt = [0_u8; 16];
    random.fill(&mut salt).map_err(|_| ())?;
    let (key, nonce) = derive(&shared, secret, point, &our_point, &salt);
    let mut sealed = told.to_vec();
    // The last record ends so.
    sealed.push(2);
    LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &key).map_err(|_| ())?)
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut sealed,
        )
        .map_err(|_| ())?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + our_point.len() + sealed.len());
    body.extend_from_slice(&salt);
    body.extend_from_slice(&RECORD.to_be_bytes());
    body.push(u8::try_from(our_point.len()).map_err(|_| ())?);
    body.extend_from_slice(&our_point);
    body.extend_from_slice(&sealed);
    Ok(body)
}

/// The key and the nonce of the content, from the secret ECDH gave,
/// `shared`, the phone's `secret` and both points, the phone's and
/// coxd's, and the `salt`, as RFC 8291 and RFC 8188 derive them with
/// HKDF-SHA-256, each of one block.
fn derive(
    shared: &[u8],
    secret: &[u8],
    phones: &[u8],
    ours: &[u8],
    salt: &[u8],
) -> ([u8; 16], [u8; 12]) {
    let first = mac(secret, &[shared]);
    let made = mac(first.as_ref(), &[b"WebPush: info\0", phones, ours, &[1]]);
    let content = mac(salt, &[made.as_ref()]);
    let key = mac(content.as_ref(), &[b"Content-Encoding: aes128gcm\0", &[1]]);
    let nonce = mac(content.as_ref(), &[b"Content-Encoding: nonce\0", &[1]]);
    let mut key_bytes = [0; 16];
    key_bytes.copy_from_slice(&key.as_ref()[..16]);
    let mut nonce_bytes = [0; 12];
    nonce_bytes.copy_from_slice(&nonce.as_ref()[..12]);
    (key_bytes, nonce_bytes)
}

#[cfg(test)]
mod tests {
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as Verifying};

    use super::*;

    fn folder() -> PathBuf {
        let folder = std::env::temp_dir().join(format!(
            "cox-push-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// A phone's keys, and its private key, to read what is pushed to it.
    fn phone(random: &SystemRandom) -> (EphemeralPrivateKey, Vec<u8>, [u8; 16]) {
        let private = EphemeralPrivateKey::generate(&ECDH_P256, random).unwrap();
        let point = private.compute_public_key().unwrap().as_ref().to_vec();
        let mut secret = [0; 16];
        random.fill(&mut secret).unwrap();
        (private, point, secret)
    }

    /// What `body` says, read as the phone's browser reads it.
    fn decrypt(private: EphemeralPrivateKey, point: &[u8], secret: &[u8], body: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        let record = u32::from_be_bytes(body[16..20].try_into().unwrap());
        assert_eq!(record, 4096);
        let length = usize::from(body[20]);
        let theirs = &body[21..21 + length];
        let shared = agreement::agree_ephemeral(
            private,
            &UnparsedPublicKey::new(&ECDH_P256, theirs),
            <[u8]>::to_vec,
        )
        .unwrap();
        let (key, nonce) = derive(&shared, secret, point, theirs, salt);
        let mut sealed = body[21 + length..].to_vec();
        let opened = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &key).unwrap())
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut sealed,
            )
            .unwrap();
        assert_eq!(opened.last(), Some(&2), "one record, the last");
        opened[..opened.len() - 1].to_vec()
    }

    #[test]
    fn pushes_what_only_the_phone_reads() {
        let random = SystemRandom::new();
        let (private, point, secret) = phone(&random);
        let body = encrypt(&point, &secret, b"{\"key\":3}", &random).unwrap();
        assert_eq!(decrypt(private, &point, &secret, &body), b"{\"key\":3}");
    }

    #[test]
    fn keeps_its_key_and_the_phones_and_signs_as_itself() {
        let folder = folder();
        let mut pushes =
            Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        let random = SystemRandom::new();
        let (private, point, secret) = phone(&random);
        let subscription = Subscription {
            endpoint: "https://fcm.googleapis.com/fcm/send/abc".to_owned(),
            keys: Keys {
                p256dh: URL_SAFE_NO_PAD.encode(&point),
                auth: URL_SAFE_NO_PAD.encode(secret),
            },
        };
        pushes.subscribe(subscription.clone()).unwrap();
        pushes.subscribe(subscription.clone()).unwrap();
        let mode = std::fs::metadata(folder.join("pushes.json"))
            .unwrap()
            .permissions();
        assert_eq!(
            std::os::unix::fs::PermissionsExt::mode(&mode) & 0o777,
            0o600,
            "where to push to a phone is the human's alone"
        );

        let again = Pushes::open_at(&folder.join("push-key"), folder.join("pushes.json")).unwrap();
        assert_eq!(again.public_key(), pushes.public_key());
        assert_eq!(again.subscriptions, vec![subscription]);

        let notice = Notice {
            key: 7,
            title: "Fix the button".to_owned(),
            body: "x".repeat(1000),
        };
        let [push] = &again.pushes(&notice)[..] else {
            panic!("one push for the one phone");
        };
        assert_eq!(push.endpoint, "https://fcm.googleapis.com/fcm/send/abc");
        let told: serde_json::Value =
            serde_json::from_slice(&decrypt(private, &point, &secret, &push.body)).unwrap();
        assert_eq!(told["key"], 7);
        assert_eq!(told["title"], "Fix the button");
        assert_eq!(told["body"].as_str().unwrap().chars().count(), TOLD);

        let (token, key) = push
            .authorization
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(key, again.public_key());
        let (signed, signature) = token.rsplit_once('.').unwrap();
        Verifying::new(&ECDSA_P256_SHA256_FIXED, decoded(key).unwrap())
            .verify(signed.as_bytes(), &decoded(signature).unwrap())
            .unwrap();
        let claims: serde_json::Value =
            serde_json::from_slice(&decoded(signed.split_once('.').unwrap().1).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["sub"], PUSHED_BY);

        let mut again = again;
        again.unsubscribe("https://fcm.googleapis.com/fcm/send/abc");
        assert!(!again.any());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn pushes_only_to_the_browsers_services() {
        for endpoint in [
            "https://fcm.googleapis.com/fcm/send/a",
            "https://updates.push.services.mozilla.com/wpush/v2/a",
            "https://web.push.apple.com/a",
            "https://wns2-par02p.notify.windows.com/w/?token=a",
        ] {
            assert_eq!(of_a_browser(endpoint), Ok(()), "{endpoint}");
        }
        for endpoint in [
            "http://fcm.googleapis.com/fcm/send/a",
            "https://127.0.0.1/a",
            "https://fcm.googleapis.com:8443/a",
            "https://evilpush.apple.com.example/a",
            "https://notpush.apple.com/a",
        ] {
            assert!(of_a_browser(endpoint).is_err(), "{endpoint}");
        }
    }
}
