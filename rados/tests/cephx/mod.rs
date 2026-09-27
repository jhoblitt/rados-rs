//! Cephx helpers shared by the `cephx_capture`, `cephx_aes256k` and
//! `cephx_policy` cluster tests: test entities with a chosen key type.
//!
//! Every auth command goes through the admin client, since it needs `auth
//! rwx`, which a test entity's `mon 'allow r'` lacks.

#![allow(dead_code)]

use std::io::Write as _;
use std::time::{Duration, Instant};

use bytes::Bytes;
use rados::{Client, Keyring};

/// How long the mon may refuse an insecure key type once it is allowed: its
/// health tick sets `mon_auth_allow_insecure_key`'s default within a
/// monmap commit, a re-election and one `mon_tick_interval` (5 s).
const INSECURE_KEY_WAIT: Duration = Duration::from_secs(30);

/// How long cleanup keeps retrying a command the mon did not answer.
const CLEANUP_WAIT: Duration = Duration::from_secs(30);

/// `rados-rs-<purpose>-<pid>-<nanos>`, unique across runs and processes.
pub fn unique(purpose: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("rados-rs-{purpose}-{}-{nanos}", std::process::id())
}

/// A cephx entity a test created, with its keyring in a temporary file.
pub struct Entity {
    pub name: String,
    pub keyring: tempfile::NamedTempFile,
    pub key_type: u16,
}

fn key_type_number(key_type: &str) -> u16 {
    match key_type {
        "aes" => rados::auth::CEPH_CRYPTO_AES,
        "aes256k" => rados::auth::CEPH_CRYPTO_AES256KRB5,
        other => panic!("no key type number for {other:?}"),
    }
}

/// Send one JSON mon command through `admin`.
pub async fn mon_command(
    admin: &Client,
    cmd: serde_json::Value,
) -> Result<rados::monclient::CommandResult, rados::MonClientError> {
    admin
        .mon_client()
        .invoke(vec![cmd.to_string()], Bytes::new())
        .await
}

/// Create `client.<unique(purpose)>` with a `key_type` key ("aes" or
/// "aes256k"), caps `mon 'allow r'` and `osd 'allow rwx pool=<test pool>'`.
///
/// The mon refuses an insecure key type with EINVAL until its health tick
/// has caught up with `auth_allowed_ciphers`, so that refusal is retried; any
/// other refusal fails at once. Every failure removes the entity best-effort
/// before panicking: the mon may have created it on an attempt whose reply
/// was lost, or with the wrong key type.
pub async fn create_entity(admin: &Client, purpose: &str, key_type: &str) -> Entity {
    let name = format!("client.{}", unique(purpose));
    let want = key_type_number(key_type);
    match try_create_entity(admin, &name, key_type, want).await {
        Ok(entity) => entity,
        Err(e) => {
            let removed = match remove_entity(admin, &name).await {
                Ok(()) => format!("auth rm {name}: ok"),
                Err(rm) => rm,
            };
            panic!("{e}; best-effort cleanup: {removed}");
        }
    }
}

async fn try_create_entity(
    admin: &Client,
    name: &str,
    key_type: &str,
    want: u16,
) -> Result<Entity, String> {
    let cmd = serde_json::json!({
        "prefix": "auth get-or-create",
        "entity": name,
        "caps": [
            "mon", "allow r",
            "osd", format!("allow rwx pool={}", crate::common::test_pool_name()),
        ],
        "key_type": key_type,
    });
    let start = Instant::now();
    let keyring_text = loop {
        let retry = match mon_command(admin, cmd.clone()).await {
            Ok(res) if res.retval == 0 && !res.outbl.is_empty() => {
                break String::from_utf8(res.outbl.to_vec())
                    .map_err(|e| format!("auth get-or-create {name}: keyring: {e}"))?;
            }
            // The mon answers a create still being proposed with no keyring;
            // the next attempt finds the entity.
            Ok(res) if res.retval == 0 => "an empty reply".to_owned(),
            Ok(res) if res.retval == -22 && res.outs.contains("insecure key type") => {
                format!("EINVAL: {}", res.outs)
            }
            Ok(res) => {
                return Err(format!(
                    "auth get-or-create {name} ({key_type}): retval {}: {}",
                    res.retval, res.outs
                ));
            }
            Err(e) => format!("{e}"),
        };
        if start.elapsed() >= INSECURE_KEY_WAIT {
            return Err(format!(
                "auth get-or-create {name} ({key_type}): still {retry} after {INSECURE_KEY_WAIT:?}"
            ));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    };

    let parsed = Keyring::from_string(&keyring_text)
        .map_err(|e| format!("keyring from auth get-or-create {name}: {e}"))?;
    let key_type_got = parsed
        .get_key(name)
        .ok_or_else(|| format!("no key for {name} in the mon's keyring"))?
        .crypto_type;
    if key_type_got != want {
        return Err(format!(
            "{name} has a type {key_type_got} key, asked for {key_type} ({want})"
        ));
    }
    let mut keyring =
        tempfile::NamedTempFile::new().map_err(|e| format!("keyring temp file: {e}"))?;
    keyring
        .write_all(keyring_text.as_bytes())
        .map_err(|e| format!("write keyring: {e}"))?;
    Ok(Entity {
        name: name.to_owned(),
        keyring,
        key_type: key_type_got,
    })
}

/// `auth rm name`, retried while the mon does not answer. Returns the
/// failure for the caller to log or collect, since it runs in cleanup.
pub async fn remove_entity(admin: &Client, name: &str) -> Result<(), String> {
    let cmd = serde_json::json!({"prefix": "auth rm", "entity": name});
    let start = Instant::now();
    loop {
        let failure = match mon_command(admin, cmd.clone()).await {
            Ok(res) if res.retval == 0 => return Ok(()),
            Ok(res) => format!("retval {}: {}", res.retval, res.outs),
            Err(e) => format!("{e}"),
        };
        if start.elapsed() >= CLEANUP_WAIT {
            return Err(format!("auth rm {name}: {failure}"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
