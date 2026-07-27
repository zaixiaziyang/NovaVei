//! Native protection for persisted provider credentials and durable transcripts.
//!
//! Settings stay as ordinary values while the application is running so the
//! provider resolver can use them without repeatedly crossing an IPC boundary.
//! Before they are written to SQLite, values identified as credentials are
//! protected with the current Windows user's DPAPI key.
//!
//! Conversation message bodies and segmented history payloads use a separate
//! DPAPI envelope so a transcript ciphertext cannot be mistaken for a settings
//! secret field, and vice versa.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

const DPAPI_MARKER: &str = "__novavei_dpapi_v1:";
const TRANSCRIPT_DPAPI_MARKER: &str = "__novavei_dpapi_msg_v1:";
const PORTABLE_MARKER: &str = "__novavei_portable_v1:";
const PORTABLE_TRANSCRIPT_MARKER: &str = "__novavei_portable_msg_v1:";
const PORTABLE_LOCAL_SERVICE_MARKER: &str = "__novavei_portable_local_v1:";
const PORTABLE_KEY_FILE: &str = "portable.json";
const PORTABLE_SCHEMA_VERSION: u8 = 2;
const PORTABLE_LEGACY_SCHEMA_VERSION: u8 = 1;
const PORTABLE_SALT_BYTES: usize = 16;
const PORTABLE_KEY_BYTES: usize = 32;
const PORTABLE_NONCE_BYTES: usize = 12;
const MAX_PORTABLE_PASSWORD_BYTES: usize = 1024;
const MIN_PORTABLE_PASSWORD_CHARS: usize = 12;
const RECOVERY_QUESTION_COUNT: usize = 3;
const MAX_RECOVERY_QUESTION_BYTES: usize = 240;
const MAX_RECOVERY_ANSWER_BYTES: usize = 1024;
const MIN_RECOVERY_QUESTION_CHARS: usize = 6;
const MIN_RECOVERY_ANSWER_CHARS: usize = 4;
const PORTABLE_VERIFIER: &[u8] = b"NovaVei portable storage verifier v1";

static PORTABLE_KEY: OnceLock<Mutex<Option<[u8; PORTABLE_KEY_BYTES]>>> = OnceLock::new();
// A pending portable data key may be temporarily installed while AppState
// hydrates encrypted rows. The renderer must not treat that intermediate state
// as an unlocked application: only a completed hydration marks it ready.
static PORTABLE_KEY_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableStorageStatus {
    pub portable: bool,
    pub initialized: bool,
    pub unlocked: bool,
    /// Recovery prompts are intentionally returned so the user can answer
    /// them before their data key has been restored. Answers never cross this
    /// boundary in the opposite direction and are never persisted as text.
    pub recovery_configured: bool,
    pub recovery_questions: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPortableKeyFile {
    schema_version: u8,
    salt: String,
    verifier: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableRecoverySetup {
    pub questions: Vec<String>,
    pub answers: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PortableKeyFile {
    schema_version: u8,
    password_salt: String,
    password_wrapped_key: String,
    recovery_salt: String,
    recovery_questions: Vec<String>,
    recovery_wrapped_key: String,
}

#[derive(Debug)]
enum PortableKeyConfig {
    Legacy(LegacyPortableKeyFile),
    Current(PortableKeyFile),
}

fn portable_key_slot() -> &'static Mutex<Option<[u8; PORTABLE_KEY_BYTES]>> {
    PORTABLE_KEY.get_or_init(|| Mutex::new(None))
}

/// Report portable storage state without exposing its path, password, salts,
/// encrypted key wrappers, or recovery answers to the WebView. The custom
/// recovery prompts are returned only so the user can answer them.
pub fn portable_storage_status() -> PortableStorageStatus {
    let portable = crate::storage::is_portable();
    let (initialized, recovery_configured, recovery_questions) = if portable {
        portable_key_file_path()
            .ok()
            .filter(|path| path.exists())
            .map(|path| match read_portable_key_config(&path) {
                Ok(PortableKeyConfig::Current(config))
                    if validate_recovery_questions(&config.recovery_questions).is_ok() =>
                {
                    (true, true, config.recovery_questions)
                }
                _ => (true, false, Vec::new()),
            })
            .unwrap_or((false, false, Vec::new()))
    } else {
        (false, false, Vec::new())
    };
    let unlocked = portable
        && PORTABLE_KEY_READY.load(Ordering::Acquire)
        && portable_key_slot()
            .lock()
            .map(|key| key.is_some())
            .unwrap_or(false);
    PortableStorageStatus {
        portable,
        initialized,
        unlocked,
        recovery_configured,
        recovery_questions,
    }
}

pub fn portable_storage_needs_unlock() -> bool {
    crate::storage::is_portable() && !portable_storage_status().unlocked
}

/// Create or unlock the portable key envelope. A random data-encryption key
/// is wrapped separately by the password and by all three recovery answers.
/// Thus resetting a forgotten password restores the existing key instead of
/// creating a key that cannot decrypt the user's older conversations.
pub fn unlock_portable_storage(
    password: &str,
    recovery_setup: Option<PortableRecoverySetup>,
) -> Result<PortableStorageStatus, String> {
    if !crate::storage::is_portable() {
        return Err("portable storage is not active for this application".to_string());
    }
    if !crate::storage::portable_marker_valid() {
        return Err("portable distribution marker is invalid; repair the portable package before unlocking data".to_string());
    }
    validate_portable_password(password)?;
    let path = portable_key_file_path()?;
    let key = match fs::read_to_string(&path) {
        Ok(contents) => match parse_portable_key_config(&contents)? {
            PortableKeyConfig::Current(config) => {
                if recovery_setup.is_some() {
                    return Err("portable recovery is already configured".to_string());
                }
                unwrap_password_key(&config, password)?
            }
            PortableKeyConfig::Legacy(config) => {
                let key = unlock_legacy_key(&config, password)?;
                if let Some(recovery) = recovery_setup {
                    let current = new_portable_key_file(&key, password, &recovery)?;
                    replace_portable_key_file(&path, &current)?;
                }
                key
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let recovery = recovery_setup.ok_or_else(|| {
                "portable recovery questions are required when creating portable data".to_string()
            })?;
            let mut key = [0_u8; PORTABLE_KEY_BYTES];
            getrandom::fill(&mut key)
                .map_err(|_| "generate portable storage key failed".to_string())?;
            let config = new_portable_key_file(&key, password, &recovery)?;
            fs::create_dir_all(crate::storage::application_data_dir())
                .map_err(|_| "create portable storage directory failed".to_string())?;
            let serialized = serde_json::to_vec_pretty(&config)
                .map_err(|_| "serialize portable storage configuration failed".to_string())?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    use std::io::Write;
                    file.write_all(&serialized)
                        .and_then(|_| file.write_all(b"\n"))
                        .and_then(|_| file.sync_all())
                        .map_err(|_| "write portable storage configuration failed".to_string())?;
                    key
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return unlock_portable_storage(password, Some(recovery))
                }
                Err(_) => return Err("create portable storage configuration failed".to_string()),
            }
        }
        Err(_) => return Err("read portable storage configuration failed".to_string()),
    };
    install_portable_key(key)?;
    Ok(portable_storage_status())
}

/// Recover the already-encrypted portable data key from all configured answers
/// and replace only its password wrapper. This deliberately never re-encrypts
/// the databases or creates a replacement key.
pub fn recover_portable_storage(
    answers: &[String],
    new_password: &str,
) -> Result<PortableStorageStatus, String> {
    if !crate::storage::is_portable() {
        return Err("portable storage is not active for this application".to_string());
    }
    if !crate::storage::portable_marker_valid() {
        return Err("portable distribution marker is invalid; repair the portable package before unlocking data".to_string());
    }
    validate_portable_password(new_password)?;
    let path = portable_key_file_path()?;
    let contents = fs::read_to_string(&path)
        .map_err(|_| "portable storage configuration is unavailable for recovery".to_string())?;
    let mut config = match parse_portable_key_config(&contents)? {
        PortableKeyConfig::Current(config) => config,
        PortableKeyConfig::Legacy(_) => {
            return Err("portable recovery is not configured; unlock with the current password and set three recovery questions first".to_string())
        }
    };
    let key = unwrap_recovery_key(&config, answers)?;
    let mut password_salt = [0_u8; PORTABLE_SALT_BYTES];
    getrandom::fill(&mut password_salt)
        .map_err(|_| "generate portable storage salt failed".to_string())?;
    let mut password_key = derive_portable_key(new_password.as_bytes(), &password_salt)?;
    config.password_salt = hex_encode(&password_salt);
    config.password_wrapped_key = encrypt_portable_bytes(&password_key, &key)?;
    password_key.fill(0);
    replace_portable_key_file(&path, &config)?;
    install_portable_key(key)?;
    Ok(portable_storage_status())
}

/// Publish a successful portable unlock only after the caller has hydrated and
/// migrated every protected durable projection.
pub fn complete_portable_storage_unlock() -> Result<(), String> {
    if portable_key_slot()
        .lock()
        .map_err(|_| "portable storage key lock is unavailable".to_string())?
        .is_none()
    {
        return Err("portable storage key is unavailable".to_string());
    }
    PORTABLE_KEY_READY.store(true, Ordering::Release);
    Ok(())
}

/// Discard a pending key when hydration or migration cannot finish. The
/// renderer remains at the password gate and protected data is inaccessible.
pub fn clear_portable_storage_key() {
    PORTABLE_KEY_READY.store(false, Ordering::Release);
    if let Ok(mut guard) = portable_key_slot().lock() {
        if let Some(mut key) = guard.take() {
            key.fill(0);
        }
    }
}

pub fn protect_settings(
    settings: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, String> {
    settings
        .iter()
        .map(|(scope, value)| {
            transform(value, true, false, false)
                .map(|value| (scope.clone(), value))
                .map_err(|error| format!("protect setting {scope}: {error}"))
        })
        .collect()
}

pub fn unprotect_settings(
    settings: &HashMap<String, Value>,
) -> Result<HashMap<String, Value>, String> {
    settings
        .iter()
        .map(|(scope, value)| {
            transform(value, false, false, false)
                .map(|value| (scope.clone(), value))
                .map_err(|error| format!("unprotect setting {scope}: {error}"))
        })
        .collect()
}

/// Protect a durable transcript string before it is written to SQLite.
///
/// Empty strings stay empty so blank assistant placeholders do not pay the
/// DPAPI round-trip. Already-protected envelopes are left untouched so a
/// migration can re-run safely.
pub fn protect_transcript_text(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Ok(value.to_string());
    }
    if crate::storage::is_portable() {
        if value.starts_with(PORTABLE_TRANSCRIPT_MARKER) {
            return Ok(value.to_string());
        }
        return portable_encrypt_string(value, PORTABLE_TRANSCRIPT_MARKER);
    }
    if value.starts_with(TRANSCRIPT_DPAPI_MARKER) {
        return Ok(value.to_string());
    }
    let bytes = protect_bytes(value.as_bytes())?;
    Ok(format!("{TRANSCRIPT_DPAPI_MARKER}{}", hex_encode(&bytes)))
}

/// Reverse [`protect_transcript_text`]. Plaintext legacy rows are returned as
/// written so pre-encryption databases still load.
pub fn unprotect_transcript_text(value: &str) -> Result<String, String> {
    if crate::storage::is_portable() && value.starts_with(PORTABLE_TRANSCRIPT_MARKER) {
        return portable_decrypt_string(value, PORTABLE_TRANSCRIPT_MARKER);
    }
    if crate::storage::is_portable() && value.starts_with(TRANSCRIPT_DPAPI_MARKER) {
        return Err(
            "portable storage contains Windows-profile encrypted data; import it on its original computer first"
                .to_string(),
        );
    }
    let Some(encoded) = value.strip_prefix(TRANSCRIPT_DPAPI_MARKER) else {
        return Ok(value.to_string());
    };
    let bytes = hex_decode(encoded)?;
    let plain = unprotect_bytes(&bytes)?;
    String::from_utf8(plain).map_err(|_| "transcript DPAPI value is not UTF-8".to_string())
}

pub fn is_protected_transcript_text(value: &str) -> bool {
    value.starts_with(TRANSCRIPT_DPAPI_MARKER) || value.starts_with(PORTABLE_TRANSCRIPT_MARKER)
}

/// Encrypt a complete LocalServices field in portable mode.  Unlike settings,
/// these values can contain ordinary user content (Memory, knowledge-base
/// text, and Cron payloads), so the full field is protected rather than only
/// credential-shaped JSON properties.  Installed mode deliberately keeps its
/// existing on-disk format.
pub fn protect_portable_local_service_text(value: &str) -> Result<String, String> {
    if !crate::storage::is_portable() {
        return Ok(value.to_string());
    }
    if value.starts_with(PORTABLE_LOCAL_SERVICE_MARKER) {
        return Ok(value.to_string());
    }
    portable_encrypt_string(value, PORTABLE_LOCAL_SERVICE_MARKER)
}

/// Reverse [`protect_portable_local_service_text`].  Portable plaintext is
/// accepted only for the one-time in-place migration performed after unlock;
/// callers must rewrite it before exposing LocalServices to IPC.
pub fn unprotect_portable_local_service_text(value: &str) -> Result<String, String> {
    if !crate::storage::is_portable() {
        return Ok(value.to_string());
    }
    if value.starts_with(PORTABLE_LOCAL_SERVICE_MARKER) {
        return portable_decrypt_string(value, PORTABLE_LOCAL_SERVICE_MARKER);
    }
    Ok(value.to_string())
}

pub fn is_portable_local_service_text(value: &str) -> bool {
    value.starts_with(PORTABLE_LOCAL_SERVICE_MARKER)
}

fn transform(
    value: &Value,
    protect: bool,
    header_context: bool,
    secret_context: bool,
) -> Result<Value, String> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| transform(item, protect, header_context, secret_context))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(object) => {
            let header_entry = header_context
                && (object.contains_key("key") || object.contains_key("name"))
                && object.contains_key("value");
            let mut output = Map::new();
            for (key, item) in object {
                let normalized = normalize_key(key);
                let is_header_name = header_entry && matches!(normalized.as_str(), "key" | "name");
                let child_header_context = header_context || is_headers_key(&normalized);
                let child_secret_context = if is_header_name {
                    false
                } else if header_context && !header_entry {
                    true
                } else {
                    secret_context
                        || is_secret_key(&normalized)
                        || is_secret_map_key(&normalized)
                        || (header_entry && normalized == "value")
                };
                output.insert(
                    key.clone(),
                    transform(item, protect, child_header_context, child_secret_context)?,
                );
            }
            Ok(Value::Object(output))
        }
        Value::String(text) if secret_context => {
            if protect {
                protect_string(text).map(Value::String)
            } else if crate::storage::is_portable() && text.starts_with(PORTABLE_MARKER) {
                portable_decrypt_string(text, PORTABLE_MARKER).map(Value::String)
            } else if crate::storage::is_portable() && text.starts_with(DPAPI_MARKER) {
                Err(
                    "portable storage contains Windows-profile encrypted data; import it on its original computer first"
                        .to_string(),
                )
            } else if text.starts_with(DPAPI_MARKER) {
                unprotect_string(text).map(Value::String)
            } else {
                Ok(Value::String(text.clone()))
            }
        }
        other => Ok(other.clone()),
    }
}

fn normalize_key(key: &str) -> String {
    key.replace(['-', '_', '.'], "").to_ascii_lowercase()
}

fn is_headers_key(key: &str) -> bool {
    matches!(key, "headers" | "customheaders")
}

fn is_secret_key(key: &str) -> bool {
    matches!(
        key,
        "apikey"
            | "key"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "authorization"
            | "xapikey"
            | "xgoogapikey"
            | "secret"
            | "clientsecret"
            | "credential"
            | "credentials"
            | "password"
            | "passphrase"
            | "privatekey"
            | "bearertoken"
    ) || key.ends_with("token")
}

fn is_secret_map_key(key: &str) -> bool {
    matches!(
        key,
        "env" | "environment" | "environmentvariables" | "environmentvars"
    )
}

fn protect_string(value: &str) -> Result<String, String> {
    if crate::storage::is_portable() {
        return portable_encrypt_string(value, PORTABLE_MARKER);
    }
    let bytes = protect_bytes(value.as_bytes())?;
    Ok(format!("{DPAPI_MARKER}{}", hex_encode(&bytes)))
}

fn unprotect_string(value: &str) -> Result<String, String> {
    if crate::storage::is_portable() && value.starts_with(PORTABLE_MARKER) {
        return portable_decrypt_string(value, PORTABLE_MARKER);
    }
    let encoded = value
        .strip_prefix(DPAPI_MARKER)
        .ok_or_else(|| "invalid DPAPI marker".to_string())?;
    let bytes = hex_decode(encoded)?;
    let plain = unprotect_bytes(&bytes)?;
    String::from_utf8(plain).map_err(|_| "DPAPI value is not UTF-8".to_string())
}

fn validate_portable_password(password: &str) -> Result<(), String> {
    if password.len() > MAX_PORTABLE_PASSWORD_BYTES
        || password.chars().count() < MIN_PORTABLE_PASSWORD_CHARS
    {
        return Err(format!(
            "portable storage password must contain at least {MIN_PORTABLE_PASSWORD_CHARS} characters"
        ));
    }
    if password.chars().any(char::is_control) {
        return Err("portable storage password cannot contain control characters".to_string());
    }
    Ok(())
}

fn validate_recovery_setup(setup: &PortableRecoverySetup) -> Result<(), String> {
    validate_recovery_questions(&setup.questions)?;
    validate_recovery_answers(&setup.answers)
}

fn validate_recovery_answers(answers: &[String]) -> Result<(), String> {
    if answers.len() != RECOVERY_QUESTION_COUNT {
        return Err("portable recovery requires exactly three answers".to_string());
    }
    for answer in answers {
        if answer.len() > MAX_RECOVERY_ANSWER_BYTES
            || answer.chars().count() < MIN_RECOVERY_ANSWER_CHARS
            || answer.chars().any(char::is_control)
        {
            return Err(format!(
                "each portable recovery answer must contain at least {MIN_RECOVERY_ANSWER_CHARS} characters"
            ));
        }
    }
    Ok(())
}

fn validate_recovery_questions(questions: &[String]) -> Result<(), String> {
    if questions.len() != RECOVERY_QUESTION_COUNT {
        return Err("portable recovery requires exactly three questions".to_string());
    }
    let mut normalized = Vec::with_capacity(questions.len());
    for question in questions {
        if question.len() > MAX_RECOVERY_QUESTION_BYTES
            || question.chars().count() < MIN_RECOVERY_QUESTION_CHARS
            || question.chars().any(char::is_control)
        {
            return Err(format!(
                "each portable recovery question must contain at least {MIN_RECOVERY_QUESTION_CHARS} characters"
            ));
        }
        let value = question.trim().to_lowercase();
        if value.is_empty() || normalized.contains(&value) {
            return Err("portable recovery questions must be different".to_string());
        }
        normalized.push(value);
    }
    Ok(())
}

fn parse_portable_key_config(contents: &str) -> Result<PortableKeyConfig, String> {
    let value: Value = serde_json::from_str(contents)
        .map_err(|_| "portable storage configuration is invalid".to_string())?;
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_u64)
        .ok_or_else(|| "portable storage configuration is invalid".to_string())?;
    match version {
        version if version == PORTABLE_LEGACY_SCHEMA_VERSION as u64 => {
            let config: LegacyPortableKeyFile = serde_json::from_value(value)
                .map_err(|_| "portable storage configuration is invalid".to_string())?;
            Ok(PortableKeyConfig::Legacy(config))
        }
        version if version == PORTABLE_SCHEMA_VERSION as u64 => {
            let config: PortableKeyFile = serde_json::from_value(value)
                .map_err(|_| "portable storage configuration is invalid".to_string())?;
            validate_recovery_questions(&config.recovery_questions)
                .map_err(|_| "portable storage configuration is invalid".to_string())?;
            Ok(PortableKeyConfig::Current(config))
        }
        _ => Err("portable storage configuration version is unsupported".to_string()),
    }
}

fn read_portable_key_config(path: &std::path::Path) -> Result<PortableKeyConfig, String> {
    let contents = fs::read_to_string(path)
        .map_err(|_| "read portable storage configuration failed".to_string())?;
    parse_portable_key_config(&contents)
}

fn unlock_legacy_key(
    config: &LegacyPortableKeyFile,
    password: &str,
) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    if config.schema_version != PORTABLE_LEGACY_SCHEMA_VERSION {
        return Err("portable storage configuration version is unsupported".to_string());
    }
    let salt = decode_portable_salt(&config.salt)?;
    let key = derive_portable_key(password.as_bytes(), &salt)?;
    let verifier = decrypt_portable_bytes(&key, &config.verifier)
        .map_err(|_| "portable storage password is incorrect".to_string())?;
    if verifier != PORTABLE_VERIFIER {
        return Err("portable storage password is incorrect".to_string());
    }
    Ok(key)
}

fn unwrap_password_key(
    config: &PortableKeyFile,
    password: &str,
) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    let salt = decode_portable_salt(&config.password_salt)?;
    let mut password_key = derive_portable_key(password.as_bytes(), &salt)?;
    let result = unwrap_portable_key(&password_key, &config.password_wrapped_key)
        .map_err(|_| "portable storage password is incorrect or data is damaged".to_string());
    password_key.fill(0);
    result
}

fn unwrap_recovery_key(
    config: &PortableKeyFile,
    answers: &[String],
) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    let salt = decode_portable_salt(&config.recovery_salt)?;
    let mut recovery_key = derive_recovery_key(answers, &salt)?;
    let result = unwrap_portable_key(&recovery_key, &config.recovery_wrapped_key)
        .map_err(|_| "portable recovery answers are incorrect or data is damaged".to_string());
    recovery_key.fill(0);
    result
}

fn new_portable_key_file(
    data_key: &[u8; PORTABLE_KEY_BYTES],
    password: &str,
    recovery: &PortableRecoverySetup,
) -> Result<PortableKeyFile, String> {
    validate_recovery_setup(recovery)?;
    let mut password_salt = [0_u8; PORTABLE_SALT_BYTES];
    let mut recovery_salt = [0_u8; PORTABLE_SALT_BYTES];
    getrandom::fill(&mut password_salt)
        .map_err(|_| "generate portable storage salt failed".to_string())?;
    getrandom::fill(&mut recovery_salt)
        .map_err(|_| "generate portable storage salt failed".to_string())?;
    let mut password_key = derive_portable_key(password.as_bytes(), &password_salt)?;
    let mut recovery_key = derive_recovery_key(&recovery.answers, &recovery_salt)?;
    let password_wrapped_key = encrypt_portable_bytes(&password_key, data_key)?;
    let recovery_wrapped_key = encrypt_portable_bytes(&recovery_key, data_key)?;
    password_key.fill(0);
    recovery_key.fill(0);
    Ok(PortableKeyFile {
        schema_version: PORTABLE_SCHEMA_VERSION,
        password_salt: hex_encode(&password_salt),
        password_wrapped_key,
        recovery_salt: hex_encode(&recovery_salt),
        recovery_questions: recovery.questions.clone(),
        recovery_wrapped_key,
    })
}

fn decode_portable_salt(encoded: &str) -> Result<Vec<u8>, String> {
    let salt =
        hex_decode(encoded).map_err(|_| "portable storage configuration is invalid".to_string())?;
    if salt.len() != PORTABLE_SALT_BYTES {
        return Err("portable storage configuration is invalid".to_string());
    }
    Ok(salt)
}

fn derive_recovery_key(
    answers: &[String],
    salt: &[u8],
) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    validate_recovery_answers(answers)?;
    let mut material = b"NovaVei portable recovery answers v1".to_vec();
    for answer in answers {
        let bytes = answer.as_bytes();
        let length: u32 = bytes
            .len()
            .try_into()
            .map_err(|_| "portable recovery answer is too large".to_string())?;
        material.extend_from_slice(&length.to_be_bytes());
        material.extend_from_slice(bytes);
    }
    let result = derive_portable_key(&material, salt);
    material.fill(0);
    result
}

fn unwrap_portable_key(
    wrapping_key: &[u8; PORTABLE_KEY_BYTES],
    wrapped: &str,
) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    let plain = decrypt_portable_bytes(wrapping_key, wrapped)?;
    let key: [u8; PORTABLE_KEY_BYTES] = plain
        .try_into()
        .map_err(|_| "portable storage ciphertext is invalid".to_string())?;
    Ok(key)
}

fn replace_portable_key_file(
    path: &std::path::Path,
    config: &PortableKeyFile,
) -> Result<(), String> {
    let serialized = serde_json::to_vec_pretty(config)
        .map_err(|_| "serialize portable storage configuration failed".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "portable storage configuration path is invalid".to_string())?;
    // This file is the only holder of the wrapped data key: losing it makes
    // the whole portable database unreadable. Keep the previous generation as
    // a sibling backup before the atomic replace so an interrupted rewrite
    // (power loss on removable media) always leaves one recoverable copy.
    if path.is_file() {
        let backup = parent.join(format!(
            "{}.bak",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("portable.json")
        ));
        fs::copy(path, &backup)
            .map_err(|_| "back up portable storage configuration failed".to_string())?;
    }
    let mut random = [0_u8; 8];
    getrandom::fill(&mut random)
        .map_err(|_| "generate portable storage temporary name failed".to_string())?;
    let temporary = parent.join(format!(".portable-{}.tmp", hex_encode(&random)));
    let write_result = (|| -> Result<(), String> {
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "write portable storage configuration failed".to_string())?;
        file.write_all(&serialized)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.sync_all())
            .map_err(|_| "write portable storage configuration failed".to_string())?;
        fs::rename(&temporary, path)
            .map_err(|_| "replace portable storage configuration failed".to_string())?;
        Ok(())
    })();
    if write_result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn install_portable_key(key: [u8; PORTABLE_KEY_BYTES]) -> Result<(), String> {
    PORTABLE_KEY_READY.store(false, Ordering::Release);
    *portable_key_slot()
        .lock()
        .map_err(|_| "portable storage key lock is unavailable".to_string())? = Some(key);
    Ok(())
}

fn portable_key_file_path() -> Result<std::path::PathBuf, String> {
    if !crate::storage::is_portable() {
        return Err("portable storage is not active for this application".to_string());
    }
    Ok(crate::storage::application_data_dir().join(PORTABLE_KEY_FILE))
}

fn derive_portable_key(password: &[u8], salt: &[u8]) -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    let params = Params::new(19 * 1024, 2, 1, Some(PORTABLE_KEY_BYTES))
        .map_err(|_| "configure portable storage encryption failed".to_string())?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0_u8; PORTABLE_KEY_BYTES];
    argon
        .hash_password_into(password, salt, &mut key)
        .map_err(|_| "derive portable storage key failed".to_string())?;
    Ok(key)
}

fn portable_key() -> Result<[u8; PORTABLE_KEY_BYTES], String> {
    portable_key_slot()
        .lock()
        .map_err(|_| "portable storage key lock is unavailable".to_string())?
        .ok_or_else(|| "unlock portable storage before accessing protected data".to_string())
}

fn portable_encrypt_string(value: &str, marker: &str) -> Result<String, String> {
    Ok(format!(
        "{marker}{}",
        encrypt_portable_bytes(&portable_key()?, value.as_bytes())?
    ))
}

fn portable_decrypt_string(value: &str, marker: &str) -> Result<String, String> {
    let encoded = value
        .strip_prefix(marker)
        .ok_or_else(|| "portable storage ciphertext is invalid".to_string())?;
    let bytes = decrypt_portable_bytes(&portable_key()?, encoded)?;
    String::from_utf8(bytes).map_err(|_| "portable storage plaintext is not UTF-8".to_string())
}

fn encrypt_portable_bytes(
    key: &[u8; PORTABLE_KEY_BYTES],
    plaintext: &[u8],
) -> Result<String, String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "initialize portable storage encryption failed".to_string())?;
    let mut nonce_bytes = [0_u8; PORTABLE_NONCE_BYTES];
    getrandom::fill(&mut nonce_bytes)
        .map_err(|_| "generate portable storage nonce failed".to_string())?;
    let nonce = Nonce::from(nonce_bytes);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| "encrypt portable storage data failed".to_string())?;
    let mut payload = Vec::with_capacity(nonce_bytes.len().saturating_add(ciphertext.len()));
    payload.extend_from_slice(&nonce_bytes);
    payload.extend_from_slice(&ciphertext);
    Ok(hex_encode(&payload))
}

fn decrypt_portable_bytes(
    key: &[u8; PORTABLE_KEY_BYTES],
    encoded: &str,
) -> Result<Vec<u8>, String> {
    let payload = hex_decode(encoded)?;
    if payload.len() <= PORTABLE_NONCE_BYTES {
        return Err("portable storage ciphertext is invalid".to_string());
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| "initialize portable storage encryption failed".to_string())?;
    let nonce_bytes: [u8; PORTABLE_NONCE_BYTES] = payload[..PORTABLE_NONCE_BYTES]
        .try_into()
        .map_err(|_| "portable storage ciphertext is invalid".to_string())?;
    let nonce = Nonce::from(nonce_bytes);
    cipher
        .decrypt(&nonce, &payload[PORTABLE_NONCE_BYTES..])
        .map_err(|_| "portable storage password is incorrect or data is damaged".to_string())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_decode(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err("invalid DPAPI ciphertext".to_string());
    }
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let high = hex_digit(pair[0]).ok_or_else(|| "invalid DPAPI ciphertext".to_string())?;
        let low = hex_digit(pair[1]).ok_or_else(|| "invalid DPAPI ciphertext".to_string())?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(windows)]
fn protect_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io;
    use std::ptr::{null, null_mut};
    use std::slice;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| "secret is too large for DPAPI".to_string())?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null_mut(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!(
            "CryptProtectData failed: {}",
            io::Error::last_os_error()
        ));
    }
    let result = if output.cbData == 0 || output.pbData.is_null() {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    if !output.pbData.is_null() {
        unsafe {
            LocalFree(output.pbData.cast());
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn unprotect_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io;
    use std::ptr::{null, null_mut};
    use std::slice;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes
            .len()
            .try_into()
            .map_err(|_| "secret is too large for DPAPI".to_string())?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(format!(
            "CryptUnprotectData failed: {}",
            io::Error::last_os_error()
        ));
    }
    let result = if output.cbData == 0 || output.pbData.is_null() {
        Vec::new()
    } else {
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    if !output.pbData.is_null() {
        unsafe {
            LocalFree(output.pbData.cast());
        }
    }
    Ok(result)
}

#[cfg(not(windows))]
fn protect_bytes(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI settings protection is only available on Windows".to_string())
}

#[cfg(not(windows))]
fn unprotect_bytes(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err("DPAPI settings protection is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn protects_only_secret_fields_and_round_trips() {
        let mut settings = HashMap::new();
        settings.insert(
            "providers".to_string(),
            serde_json::json!([{
                "id": "openai",
                "model": "gpt-4.1",
                "apiKey": "provider-secret",
                "customHeaders": [{"key": "X-Client", "value": "header-secret"}]
            }]),
        );
        settings.insert(
            "ssh".to_string(),
            serde_json::json!([{"host": "example", "privateKey": "ssh-secret", "port": 22}]),
        );
        let protected = protect_settings(&settings).expect("DPAPI should protect settings");
        assert_eq!(
            protected["providers"][0]["model"],
            serde_json::json!("gpt-4.1")
        );
        assert!(protected["providers"][0]["apiKey"]
            .as_str()
            .is_some_and(|value| value.starts_with(DPAPI_MARKER)));
        assert!(protected["providers"][0]["customHeaders"][0]["value"]
            .as_str()
            .is_some_and(|value| value.starts_with(DPAPI_MARKER)));
        assert_eq!(
            protected["providers"][0]["customHeaders"][0]["key"],
            serde_json::json!("X-Client")
        );
        assert_eq!(unprotect_settings(&protected).unwrap(), settings);
    }

    #[cfg(windows)]
    #[test]
    fn a_literal_marker_from_the_renderer_is_stored_as_a_secret() {
        let value = serde_json::json!({"apiKey": format!("{DPAPI_MARKER}deadbeef")});
        let mut settings = HashMap::new();
        settings.insert("providers".to_string(), value.clone());
        let protected = protect_settings(&settings).unwrap();
        assert_ne!(protected["providers"], value);
        assert_eq!(unprotect_settings(&protected).unwrap(), settings);
    }

    #[cfg(windows)]
    #[test]
    fn transcript_text_round_trips_under_a_distinct_marker() {
        let plain = "session prompt with token=abc";
        let protected = protect_transcript_text(plain).expect("protect transcript");
        assert!(protected.starts_with(TRANSCRIPT_DPAPI_MARKER));
        assert!(!protected.starts_with(DPAPI_MARKER));
        assert_eq!(unprotect_transcript_text(&protected).unwrap(), plain);
        assert_eq!(
            protect_transcript_text(&protected).unwrap(),
            protected,
            "already-protected envelopes must stay idempotent"
        );
        assert_eq!(
            unprotect_transcript_text("legacy plaintext").unwrap(),
            "legacy plaintext"
        );
        assert_eq!(protect_transcript_text("").unwrap(), "");
    }

    #[test]
    fn portable_envelope_round_trips_and_rejects_tampering() {
        let key = [7_u8; PORTABLE_KEY_BYTES];
        let encrypted = encrypt_portable_bytes(&key, b"portable secret").unwrap();
        assert_eq!(
            decrypt_portable_bytes(&key, &encrypted).unwrap(),
            b"portable secret"
        );
        let mut tampered = encrypted.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'0' { b'1' } else { b'0' };
        assert!(decrypt_portable_bytes(&key, std::str::from_utf8(&tampered).unwrap()).is_err());
    }

    #[test]
    fn portable_envelope_preserves_the_existing_nonce_ciphertext_layout() {
        // NIST AES-256-GCM vector: a 12-byte nonce followed by ciphertext and
        // authentication tag. Portable data created before the dependency
        // upgrade uses this exact byte layout.
        let key = [0_u8; PORTABLE_KEY_BYTES];
        let encoded = concat!(
            "000000000000000000000000",
            "cea7403d4d606b6e074ec5d3baf39d18",
            "d0d1c8a799996bf0265b98b5d48ab919"
        );

        assert_eq!(
            decrypt_portable_bytes(&key, encoded).unwrap(),
            vec![0_u8; 16]
        );
    }

    #[test]
    fn portable_password_policy_rejects_short_or_control_values() {
        assert!(validate_portable_password("short").is_err());
        assert!(validate_portable_password("twelve\nchars").is_err());
        assert!(validate_portable_password("twelve chars").is_ok());
    }

    #[test]
    fn recovery_answers_restore_the_same_data_key_after_a_password_reset() {
        let data_key = [19_u8; PORTABLE_KEY_BYTES];
        let password = "initial portable password";
        let recovery = PortableRecoverySetup {
            questions: vec![
                "Where did I first meet my mentor?".to_string(),
                "What was my first project codename?".to_string(),
                "Which city did I visit in spring?".to_string(),
            ],
            answers: vec![
                "coffee shop".to_string(),
                "north star".to_string(),
                "springfield".to_string(),
            ],
        };
        let mut config = new_portable_key_file(&data_key, password, &recovery).unwrap();
        assert_eq!(unwrap_password_key(&config, password).unwrap(), data_key);
        assert_eq!(
            unwrap_recovery_key(&config, &recovery.answers).unwrap(),
            data_key
        );
        assert!(unwrap_recovery_key(
            &config,
            &[
                "coffee shop".to_string(),
                "wrong answer".to_string(),
                "springfield".to_string()
            ],
        )
        .is_err());

        let replacement_password = "replacement portable password";
        let replacement_salt = [31_u8; PORTABLE_SALT_BYTES];
        let mut replacement_key =
            derive_portable_key(replacement_password.as_bytes(), &replacement_salt).unwrap();
        config.password_salt = hex_encode(&replacement_salt);
        config.password_wrapped_key = encrypt_portable_bytes(&replacement_key, &data_key).unwrap();
        replacement_key.fill(0);

        assert_eq!(
            unwrap_password_key(&config, replacement_password).unwrap(),
            data_key
        );
        assert_eq!(
            unwrap_recovery_key(&config, &recovery.answers).unwrap(),
            data_key
        );
        let serialized = serde_json::to_string(&config).unwrap();
        assert!(!serialized.contains("coffee shop"));
        assert!(serialized.contains("Where did I first meet my mentor?"));
    }

    #[test]
    fn recovery_setup_requires_three_distinct_questions_and_answers() {
        let duplicate_questions = PortableRecoverySetup {
            questions: vec![
                "Which color do I prefer?".to_string(),
                "Which color do I prefer?".to_string(),
                "Which number is special to me?".to_string(),
            ],
            answers: vec!["blue".to_string(), "blue".to_string(), "seven".to_string()],
        };
        assert!(validate_recovery_setup(&duplicate_questions).is_err());
        assert!(validate_recovery_answers(&["one".to_string()]).is_err());
    }

    #[test]
    fn legacy_password_key_can_be_wrapped_for_three_question_recovery() {
        let password = "legacy portable password";
        let salt = [41_u8; PORTABLE_SALT_BYTES];
        let legacy_key = derive_portable_key(password.as_bytes(), &salt).unwrap();
        let contents = serde_json::json!({
            "schemaVersion": PORTABLE_LEGACY_SCHEMA_VERSION,
            "salt": hex_encode(&salt),
            "verifier": encrypt_portable_bytes(&legacy_key, PORTABLE_VERIFIER).unwrap(),
        })
        .to_string();
        let PortableKeyConfig::Legacy(legacy) = parse_portable_key_config(&contents).unwrap()
        else {
            panic!("legacy config should remain readable")
        };
        let unlocked_key = unlock_legacy_key(&legacy, password).unwrap();
        let recovery = PortableRecoverySetup {
            questions: vec![
                "Which street did I grow up on?".to_string(),
                "What was my favorite school subject?".to_string(),
                "Which book changed my mind?".to_string(),
            ],
            answers: vec![
                "elm avenue".to_string(),
                "physics".to_string(),
                "dune novel".to_string(),
            ],
        };
        let upgraded = new_portable_key_file(&unlocked_key, password, &recovery).unwrap();
        assert_eq!(
            unwrap_password_key(&upgraded, password).unwrap(),
            legacy_key
        );
        assert_eq!(
            unwrap_recovery_key(&upgraded, &recovery.answers).unwrap(),
            legacy_key
        );
    }
}
