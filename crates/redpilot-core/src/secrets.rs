//! 密钥存储：优先系统钥匙串（macOS Keychain / Windows 凭据管理器），
//! 钥匙串不可用时退回数据目录下权限受限的 secrets.json。
//! 环境变量 `REDPILOT_REDMINE_KEY` / `REDPILOT_AI_KEY` 可覆盖（便于无界面的 MCP 部署）。

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

const SERVICE: &str = "com.redpilot.app";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    RedmineApiKey,
    AiApiKey,
}

impl SecretKind {
    fn account(self) -> &'static str {
        match self {
            SecretKind::RedmineApiKey => "redmine-api-key",
            SecretKind::AiApiKey => "ai-api-key",
        }
    }

    fn env_var(self) -> &'static str {
        match self {
            SecretKind::RedmineApiKey => "REDPILOT_REDMINE_KEY",
            SecretKind::AiApiKey => "REDPILOT_AI_KEY",
        }
    }
}

pub struct SecretStore {
    fallback_file: PathBuf,
}

impl SecretStore {
    pub fn new(fallback_file: PathBuf) -> Self {
        Self { fallback_file }
    }

    pub fn get(&self, kind: SecretKind) -> Result<Option<String>> {
        if let Ok(v) = std::env::var(kind.env_var()) {
            if !v.trim().is_empty() {
                return Ok(Some(v.trim().to_string()));
            }
        }
        match keyring::Entry::new(SERVICE, kind.account()).and_then(|e| e.get_password()) {
            Ok(v) => return Ok(Some(v)),
            Err(keyring::Error::NoEntry) => {}
            Err(e) => tracing::warn!("读取钥匙串失败，改用本地文件: {e}"),
        }
        Ok(self.read_file()?.remove(kind.account()))
    }

    pub fn set(&self, kind: SecretKind, value: &str) -> Result<()> {
        let value = value.trim();
        if value.is_empty() {
            return self.delete(kind);
        }
        match keyring::Entry::new(SERVICE, kind.account()).and_then(|e| e.set_password(value)) {
            Ok(()) => {
                // 钥匙串写入成功后清理旧的文件副本
                let mut map = self.read_file()?;
                if map.remove(kind.account()).is_some() {
                    self.write_file(&map)?;
                }
                Ok(())
            }
            Err(e) => {
                tracing::warn!("写入钥匙串失败，改用本地文件: {e}");
                let mut map = self.read_file()?;
                map.insert(kind.account().to_string(), value.to_string());
                self.write_file(&map)
            }
        }
    }

    pub fn delete(&self, kind: SecretKind) -> Result<()> {
        if let Ok(e) = keyring::Entry::new(SERVICE, kind.account()) {
            let _ = e.delete_credential();
        }
        let mut map = self.read_file()?;
        if map.remove(kind.account()).is_some() {
            self.write_file(&map)?;
        }
        Ok(())
    }

    fn read_file(&self) -> Result<HashMap<String, String>> {
        if !self.fallback_file.exists() {
            return Ok(HashMap::new());
        }
        let text = std::fs::read_to_string(&self.fallback_file)?;
        Ok(serde_json::from_str(&text).unwrap_or_default())
    }

    fn write_file(&self, map: &HashMap<String, String>) -> Result<()> {
        std::fs::write(&self.fallback_file, serde_json::to_string(map)?)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.fallback_file, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}
