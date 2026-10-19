//! Optional settings are ordinary chezmoi data + a native template helper.
//! There is no inverse-template engine or independent renderer.
use crate::{
    backend,
    config::{Config, atomic_write},
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    Shared,
    Os,
    Machine,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Change {
    pub key: String,
    pub scope: Scope,
    pub value: Option<String>,
    pub identity: String,
}
pub const HELPER: &str = "{{- $s := index .root.dot_settings .key -}}\n{{- $value := $s.shared -}}\n{{- if hasKey $s.os .root.chezmoi.os -}}{{- $value = index $s.os .root.chezmoi.os -}}{{- end -}}\n{{- if hasKey $s.machines .root.chezmoi.hostname -}}{{- $value = index $s.machines .root.chezmoi.hostname -}}{{- end -}}\n{{- $value -}}\n";
pub fn read(c: &Config) -> Result<Value> {
    let path = c.source.join(".chezmoidata.json");
    if path.exists() {
        let v: Value = serde_json::from_slice(&fs::read(path)?)?;
        if !v.is_object() {
            bail!(".chezmoidata.json must be an object");
        }
        Ok(v)
    } else {
        Ok(json!({}))
    }
}
pub fn identity(c: &Config, scope: Scope) -> Result<String> {
    match scope {
        Scope::Shared => Ok("shared".into()),
        Scope::Os => Ok(
            backend::chez(c, &["execute-template", "{{ .chezmoi.os }}"])?
                .trim()
                .into(),
        ),
        Scope::Machine => Ok(
            backend::chez(c, &["execute-template", "{{ .chezmoi.hostname }}"])?
                .trim()
                .into(),
        ),
    }
}
pub fn update(c: &Config, change: &Change, write: bool) -> Result<String> {
    if change.key.is_empty() || change.key.chars().any(char::is_control) {
        bail!("Setting key is empty or contains controls");
    }
    let mut data = read(c)?;
    let old = data.to_string();
    if data.get("dot_settings").is_none() {
        data["dot_settings"] = json!({});
    }
    let settings = data["dot_settings"]
        .as_object_mut()
        .context("dot_settings is not an object")?;
    if !settings.contains_key(&change.key) {
        if change.value.is_none() {
            bail!("Setting does not exist");
        }
        settings.insert(
            change.key.clone(),
            json!({"shared":"","os":{},"machines":{}}),
        );
    }
    let s = settings.get_mut(&change.key).unwrap();
    match change.scope {
        Scope::Shared => {
            s["shared"] = json!(
                change
                    .value
                    .as_ref()
                    .context("Shared value cannot be removed; remove an override to inherit")?
            );
        }
        Scope::Os | Scope::Machine => {
            let kind = if matches!(change.scope, Scope::Os) {
                "os"
            } else {
                "machines"
            };
            let map = s[kind].as_object_mut().context("Invalid override map")?;
            if let Some(v) = &change.value {
                map.insert(change.identity.clone(), json!(v));
            } else {
                map.remove(&change.identity);
            }
        }
    }
    let helper = c.source.join(".chezmoitemplates/dot-setting");
    if helper.exists() && fs::read_to_string(&helper)? != HELPER {
        bail!("Existing dot-setting helper differs; preserve and resolve it manually");
    }
    let new = serde_json::to_string_pretty(&data)?;
    if write {
        atomic_write(&helper, HELPER.as_bytes())?;
        atomic_write(&c.source.join(".chezmoidata.json"), new.as_bytes())?;
    }
    Ok(format!(
        "Native chezmoi setting · {:?} · {}\nBefore:\n{}\nAfter:\n{}\n\nUse in a .tmpl source (edit source to bind it explicitly):\n{{{{ template \"dot-setting\" (dict \"root\" . \"key\" {}) }}}}\nValues are literal strings; add format-appropriate quoting in the template.",
        change.scope,
        change.identity,
        old,
        new,
        serde_json::to_string(&change.key)?
    ))
}
pub fn describe(c: &Config) -> Result<String> {
    let data = read(c)?;
    Ok(format!(
        "Shared → OS → hostname overrides\n{}\n\nBindings use the native dot-setting helper. Arbitrary existing templates retain their semantics. Machine overrides use chezmoi's hostname, independent of dot's display label.",
        serde_json::to_string_pretty(data.get("dot_settings").unwrap_or(&Value::Null))?
    ))
}
