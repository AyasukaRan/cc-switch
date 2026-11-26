//! A direct provider may own the complete config.toml. Keep the managed
//! document and its outgoing provider fields together until managed mode resumes.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, TableLike, Value as TomlValue};

use crate::app_config::AppType;
use crate::codex_config::get_codex_config_path;
use crate::database::Database;
use crate::error::AppError;
use crate::live::engine::{read_current, sha256_hex, DeviceStore, LiveFile};
use crate::live::patch::toml::{parse, value_text, TomlDocPatch};
use crate::live::patch::{Guarded, LivePatch, WholeFile};
use crate::mode::operation::{AppWrite, FileChange};
use crate::mode::state::{op, PendingTarget};
use crate::provider::Provider;

use super::codex_direct::guarded;

pub(crate) fn text(provider: &Provider) -> Result<Option<&str>, AppError> {
    if !provider.uses_codex_full_config() {
        return Ok(None);
    }
    let text = provider
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AppError::Config("Codex config.toml must be text".into()))?;
    let doc = parse(&get_codex_config_path(), Some(text.as_bytes()))?;
    validate_routes(doc.as_table())?;
    Ok(Some(text))
}

/// The direct pointer identifies imported files as well as files installed by
/// CC Switch. During proxy takeover config.toml holds a route contract.
pub(super) fn active_text(db: &Database, provider: &Provider) -> Result<Option<String>, AppError> {
    if !provider.uses_codex_full_config() || crate::mode::current::is_proxy(&AppType::Codex) {
        return Ok(None);
    }
    let current = crate::mode::current::provider_for(
        db,
        &AppType::Codex,
        crate::mode::current::Purpose::Direct,
    )?;
    if current.as_deref() != Some(provider.id.as_str()) {
        return Ok(None);
    }
    let path = get_codex_config_path();
    let state = Base::read(&DeviceStore::for_device(), &path)?;
    if state
        .saved
        .as_ref()
        .and_then(|v| v.active_id.as_deref())
        .is_some_and(|id| id != provider.id)
    {
        return Err(AppError::Conflict(
            "Codex config.toml belongs to a different active provider".into(),
        ));
    }
    crate::codex_config::read_optional_codex_config_text()
}

/// Only the config text belongs to the outgoing card. A concurrent DB edit
/// must never be lost when adopting a change made in Codex itself.
pub(crate) fn adopt_active(db: &Database, provider: &Provider) -> Result<Option<String>, AppError> {
    if !provider.uses_codex_full_config() {
        return Ok(None);
    }
    let write = AppWrite::begin(db, AppType::Codex.as_str())?;
    let Some(disk) = active_text(db, provider)? else {
        return Ok(None);
    };
    let stored = provider
        .settings_config
        .get("config")
        .and_then(|v| v.as_str());
    if stored != Some(&disk) {
        db.adopt_codex_config_text(&provider.id, stored, &disk)?;
    }
    let path = get_codex_config_path();
    let mut base = Base::read(&write.store, &path)?;
    // A directly imported document has no preceding managed document. The
    // first shared MCP/settings edits belong in a separate managed base.
    let saved = base.saved.get_or_insert_with(|| SavedBase {
        managed: None,
        outgoing: Vec::new(),
        active_hash: String::new(),
        active_id: None,
    });
    let hash = sha256_hex(disk.as_bytes());
    if saved.active_id.as_deref() != Some(provider.id.as_str()) || saved.active_hash != hash {
        saved.active_id = Some(provider.id.clone());
        saved.active_hash = hash;
        let patch = guarded(base.pre.as_deref(), WholeFile::Write(encode(saved)?));
        write.run(
            op::APPLY,
            &[FileChange {
                file: LiveFile::private(&base.path),
                patch: &patch,
            }],
            PendingTarget::default(),
        )?;
    }
    Ok(Some(disk))
}

/// Three-way comparison for the whole TOML document. Metadata-only saves
/// leave the newest disk document in place.
pub(super) fn edited_text<'a>(
    base: &str,
    edited: &'a str,
    disk: &'a str,
    policy: super::claude_editor::ConflictPolicy,
) -> Result<&'a str, AppError> {
    if edited == base || edited == disk {
        return Ok(disk);
    }
    if disk == base {
        return Ok(edited);
    }
    match policy {
        super::claude_editor::ConflictPolicy::KeepMine => Ok(edited),
        super::claude_editor::ConflictPolicy::KeepTheirs => Ok(disk),
        super::claude_editor::ConflictPolicy::Refuse => {
            Err(crate::live::patch::LiveWriteError::EditConflict {
                path: get_codex_config_path(),
                keys: vec!["config.toml".into()],
            }
            .into())
        }
    }
}

fn string<'a>(table: &'a dyn TableLike, key: &str) -> Option<&'a str> {
    table
        .get(key)
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn is_openai_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| {
        url.scheme() == "https"
            && url.host_str().is_some_and(|host| {
                matches!(host, "api.openai.com" | "chatgpt.com" | "chat.openai.com")
                    || host.ends_with(".api.openai.com")
            })
            && url.port_or_known_default() == Some(443)
    })
}

fn auth_error(key: &str) -> AppError {
    AppError::localized(
        "provider.codex.full_config.official_auth_fallback",
        format!("完整 config.toml 中的 {key} 会让 Codex 把官方登录凭据发给第三方地址。请在第三方路由表内设置 env_key 或 experimental_bearer_token，并移除内置 OpenAI 地址的改道。"),
        format!("{key} in the full config.toml would send the official login to a third-party endpoint. Configure an env_key or experimental_bearer_token in the third-party route and remove any reroute of the built-in OpenAI endpoint."),
    )
}

fn validate_routes(root: &dyn TableLike) -> Result<(), AppError> {
    // A profile can select any top-level provider, including the built-in
    // OpenAI provider with a top-level base URL override.
    let check_profile = |profile: &dyn TableLike| -> Result<(), AppError> {
        let selector = string(profile, "model_provider")
            .or_else(|| string(root, "model_provider"))
            .unwrap_or("openai");
        if selector == "openai"
            && string(root, "openai_base_url").is_some_and(|url| !is_openai_url(url))
        {
            return Err(auth_error("openai_base_url"));
        }
        if string(profile, "chatgpt_base_url").is_some_and(|url| !is_openai_url(url)) {
            return Err(auth_error("chatgpt_base_url"));
        }
        Ok(())
    };
    check_profile(root)?;
    if let Some(profiles) = root.get("profiles").and_then(Item::as_table_like) {
        for (_, item) in profiles.iter() {
            if let Some(profile) = item.as_table_like() {
                check_profile(profile)?;
            }
        }
    }
    if let Some(providers) = root.get("model_providers").and_then(Item::as_table_like) {
        for (id, item) in providers.iter() {
            let Some(route) = item.as_table_like() else {
                continue;
            };
            let needs_login = id == "openai"
                || route.get("requires_openai_auth").and_then(Item::as_bool) == Some(true);
            if needs_login
                && string(route, "env_key").is_none()
                && string(route, "experimental_bearer_token").is_none()
            {
                for endpoint in ["base_url", "model_catalog_url"] {
                    if string(route, endpoint).is_some_and(|url| !is_openai_url(url)) {
                        return Err(auth_error(&format!("model_providers.{id}.{endpoint}")));
                    }
                }
            }
        }
    }
    Ok(())
}

type OutgoingFields = Vec<(String, TomlValue)>;

#[derive(Serialize, Deserialize)]
struct SavedBase {
    managed: Option<String>,
    outgoing: Vec<(String, String)>,
    active_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_id: Option<String>,
}

pub(super) struct Base {
    pub path: PathBuf,
    pre: Option<Vec<u8>>,
    saved: Option<SavedBase>,
}

impl Base {
    pub fn read(store: &DeviceStore, config: &Path) -> Result<Self, AppError> {
        let id = sha256_hex(config.as_os_str().as_encoded_bytes());
        let path = store.file(&format!("codex-full-config-{id}.json"));
        let pre = read_current(&path)?;
        let saved = pre
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()
            .map_err(|e| {
                AppError::Message(format!(
                    "Cannot read Codex managed configuration backup {}: {e}",
                    path.display()
                ))
            })?;
        Ok(Self { path, pre, saved })
    }

    pub fn managed(&self, path: &Path) -> Result<Option<Vec<u8>>, AppError> {
        match &self.saved {
            Some(saved) => Ok(saved.managed.as_deref().map(|s| s.as_bytes().to_vec())),
            None => Ok(read_current(path)?),
        }
    }

    pub fn outgoing(&self) -> Result<Option<OutgoingFields>, AppError> {
        self.saved
            .as_ref()
            .map(|saved| {
                saved
                    .outgoing
                    .iter()
                    .map(|(key, value)| {
                        value
                            .parse::<TomlValue>()
                            .map(|v| (key.clone(), v))
                            .map_err(|e| {
                                AppError::Config(format!("Invalid saved Codex field {key}: {e}"))
                            })
                    })
                    .collect()
            })
            .transpose()
    }

    /// Both files enter the same pending operation. Refuse to overwrite edits
    /// made to an active full file unless the user saves that exact text.
    pub fn transition(
        &mut self,
        path: &Path,
        target: Option<&str>,
        target_id: Option<&str>,
        expected_live: Option<&str>,
        managed_patch: &dyn TomlDocPatch,
        outgoing: &[(String, TomlValue)],
    ) -> Result<Option<(Guarded, Guarded)>, AppError> {
        if self.saved.is_none() && target.is_none() && expected_live.is_none() {
            return Ok(None);
        }
        let live = read_current(path)?;
        if expected_live.is_some_and(|expected| Some(expected.as_bytes()) != live.as_deref()) {
            return Err(crate::live::patch::LiveWriteError::Conflict {
                path: path.to_path_buf(),
            }
            .into());
        }
        if let Some(saved) = &self.saved {
            let changed =
                crate::live::engine::digest(live.as_deref()).as_ref() != Some(&saved.active_hash);
            // Browser text editors can normalize pasted Windows line endings.
            let adopts_disk = || {
                target
                    .zip(
                        live.as_deref()
                            .and_then(|bytes| std::str::from_utf8(bytes).ok()),
                    )
                    .is_some_and(|(new, disk)| {
                        new == disk || new.replace("\r\n", "\n") == disk.replace("\r\n", "\n")
                    })
            };
            if changed && expected_live.is_none() && !adopts_disk() {
                return Err(AppError::localized(
                    "provider.codex.full_config.changed",
                    format!("{} 在 CC Switch 之外被修改了。请先在当前供应商的完整配置框中粘贴并保存磁盘上的内容，或恢复磁盘文件后重试。", path.display()),
                    format!("{} changed outside CC Switch. Paste and save the on-disk text in the active provider's full configuration editor, or restore the file before retrying.", path.display()),
                ));
            }
        }
        let (contents, snapshot) = match target {
            Some(new) => {
                if self.saved.is_none() {
                    parse(path, live.as_deref())?;
                    let managed = live
                        .as_ref()
                        .map(|bytes| String::from_utf8(bytes.clone()))
                        .transpose()
                        .map_err(|e| AppError::Config(e.to_string()))?;
                    self.saved = Some(SavedBase {
                        managed,
                        outgoing: outgoing
                            .iter()
                            .map(|(key, value)| (key.clone(), value_text(value)))
                            .collect(),
                        active_hash: String::new(),
                        active_id: None,
                    });
                }
                let saved = self.saved.as_mut().expect("initialized above");
                saved.active_hash = sha256_hex(new.as_bytes());
                saved.active_id = target_id.map(str::to_owned);
                (
                    WholeFile::Write(new.as_bytes().to_vec()),
                    WholeFile::Write(encode(saved)?),
                )
            }
            None => {
                let original = match &self.saved {
                    Some(saved) => saved.managed.as_deref().map(str::as_bytes),
                    None => live.as_deref(), // Imported full file; no managed snapshot precedes it.
                };
                let mut doc = parse(path, original)?;
                let before = doc.to_string();
                managed_patch.apply_to(path, &mut doc)?;
                let after = doc.to_string();
                // toml_edit normalizes CRLF and the final newline on output.
                // A no-op restores the original bytes.
                let contents = if before == after {
                    original.map_or(WholeFile::Delete, |bytes| WholeFile::Write(bytes.to_vec()))
                } else {
                    WholeFile::Write(after.into_bytes())
                };
                (contents, WholeFile::Delete)
            }
        };
        Ok(Some((
            guarded(live.as_deref(), contents),
            guarded(self.pre.as_deref(), snapshot),
        )))
    }
}

fn encode(base: &SavedBase) -> Result<Vec<u8>, AppError> {
    serde_json::to_vec(base).map_err(|e| AppError::Message(e.to_string()))
}

pub(super) fn managed_base() -> Result<(DocumentMut, Option<OutgoingFields>), AppError> {
    let path = get_codex_config_path();
    let base = Base::read(&DeviceStore::for_device(), &path)?;
    let doc = parse(&path, base.managed(&path)?.as_deref())?;
    Ok((doc, base.outgoing()?))
}

pub(crate) fn managed_text() -> Result<String, AppError> {
    let path = get_codex_config_path();
    let base = Base::read(&DeviceStore::for_device(), &path)?;
    String::from_utf8(base.managed(&path)?.unwrap_or_default())
        .map_err(|err| AppError::Config(format!("Invalid UTF-8 in {}: {err}", path.display())))
}

/// Global edits from a managed card are scoped to the managed base, even
/// while another card owns the live file.
pub(crate) fn write_managed_edits(
    write: &AppWrite<'_>,
    edits: &dyn LivePatch,
) -> Result<(), AppError> {
    let path = get_codex_config_path();
    let mut base = Base::read(&write.store, &path)?;
    let snapshot;
    let (file, patch) = if let Some(saved) = &mut base.saved {
        let edited = edits.apply(&path, saved.managed.as_deref().map(str::as_bytes))?;
        saved.managed =
            Some(String::from_utf8(edited).map_err(|e| AppError::Config(e.to_string()))?);
        snapshot = guarded(base.pre.as_deref(), WholeFile::Write(encode(saved)?));
        (LiveFile::private(base.path), &snapshot as &dyn LivePatch)
    } else {
        (LiveFile::private(path), edits)
    };
    write.run(
        op::APPLY,
        &[FileChange { file, patch }],
        PendingTarget::default(),
    )?;
    Ok(())
}

/// MCP's public write helpers also run without an AppState. Serialize their
/// single-file writes with provider transactions and leave recovery to AppWrite.
pub(crate) fn edit_managed_text(
    edit: impl FnOnce(Option<&str>) -> Result<Option<String>, AppError>,
) -> Result<(), AppError> {
    let path = get_codex_config_path();
    let guard = crate::live::engine::lock_app("codex");
    let store = DeviceStore::for_device();
    if crate::mode::state::pending(&store, guard.app())?.is_some() {
        return Err(AppError::Conflict(
            "A Codex configuration write is pending recovery".into(),
        ));
    }
    let mut base = Base::read(&store, &path)?;
    let content = base.managed(&path)?;
    let before = content
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|error| AppError::Config(error.to_string()))?;
    let Some(after) = edit(before)? else {
        return Ok(());
    };
    if before == Some(after.as_str()) {
        return Ok(());
    }
    let (file, patch) = match &mut base.saved {
        Some(saved) => {
            saved.managed = Some(after);
            (
                base.path,
                guarded(base.pre.as_deref(), WholeFile::Write(encode(saved)?)),
            )
        }
        None => (
            path,
            guarded(content.as_deref(), WholeFile::Write(after.into_bytes())),
        ),
    };
    let bytes = patch.apply(&file, read_current(&file)?.as_deref())?;
    crate::config::atomic_write_private(&file, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_full_config_recovers_the_snapshot_after_partial_publication() {
        use crate::live::engine::lock_app;
        use crate::live::patch::toml::TomlPatch;
        use crate::mode::operation::{self, failpoint};

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = "# managed\r\nmodel = 'prior'";
        std::fs::write(&path, original).unwrap();
        let store = DeviceStore::at(dir.path().join("device"));
        let guard = lock_app(&format!("codex-full-test-{}", dir.path().display()));
        let mut base = Base::read(&store, &path).unwrap();
        let raw = "# full\nmodel = 'custom'\n";
        let (config, snapshot) = base
            .transition(
                &path,
                Some(raw),
                Some("test"),
                None,
                &TomlPatch::default(),
                &[],
            )
            .unwrap()
            .unwrap();
        let changes = [
            FileChange {
                file: LiveFile::private(&path),
                patch: &config,
            },
            FileChange {
                file: LiveFile::private(&base.path),
                patch: &snapshot,
            },
        ];
        failpoint::crash_at(Some("published:0"));
        let result = operation::run(
            &store,
            &guard,
            op::SWITCH,
            &changes,
            PendingTarget::default(),
            &|_| Ok(()),
        );
        failpoint::crash_at(None);
        assert!(result.is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), raw);
        assert!(!base.path.exists());
        operation::recover(&store, &guard, &|_| Ok(())).unwrap();
        let mut recovered = Base::read(&store, &path).unwrap();
        let (restore, remove) = recovered
            .transition(&path, None, None, None, &TomlPatch::default(), &[])
            .unwrap()
            .unwrap();
        assert_eq!(restore.then, WholeFile::Write(original.as_bytes().to_vec()));
        assert_eq!(remove.then, WholeFile::Delete);
    }

    #[test]
    fn codex_full_config_uses_only_the_documents_credential_sources() {
        let validate =
            |text: &str| validate_routes(text.parse::<DocumentMut>().unwrap().as_table());
        let route = "[model_providers.team]\nbase_url = 'https://team.example/v1'\n";
        assert!(validate(&format!("{route}auth = {{ command = 'get-key' }}")).is_ok());
        for own_key in [
            "env_key = 'TEAM_KEY'",
            "experimental_bearer_token = 'private-key'",
        ] {
            assert!(validate(&format!("{route}requires_openai_auth = true\n{own_key}")).is_ok());
        }
        assert!(validate(&format!(
            "{route}requires_openai_auth = true\nenv_key = '   '"
        ))
        .is_err());
        assert!(validate("openai_base_url = 'https://api.openai.com.evil.test/v1'").is_err());
        assert!(validate("openai_base_url = 'https://eu.api.openai.com/v1'").is_ok());
        assert!(validate("[model_providers.team]\nrequires_openai_auth = true\nmodel_catalog_url = 'https://team.example/models'").is_err());
    }
}
