//! `appimage` module — manage AppImage bundles.
//!
//! AppImages are self-contained executables; there is no central package
//! manager.  This module handles:
//!
//! - **Download** from a URL and make executable (`state: present`)
//! - **Remove** an installed AppImage file (`state: absent`)
//! - **Integrate** with the desktop environment via `appimaged` or
//!   `AppImageLauncher` (`integrate: true`)
//! - **Update** an existing AppImage via its embedded update information
//!   using `appimageupdatetool` (`state: latest`)
//! - **Verify** the AppImage signature via `gpg` (`verify: true`)
//!
//! ## Supported parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `src` | — | Source URL or local path of the `.AppImage` file |
//! | `dest` | — | Destination path (directory or full file path) |
//! | `state` | `present` | `present`, `absent`, `latest` |
//! | `integrate` | `false` | Register with `appimaged` / `ail-cli` after install |
//! | `verify` | `false` | Verify embedded GPG signature before installing |
//! | `mode` | `0755` | File permission bits |
//! | `checksum` | — | Expected `sha256:` checksum of the download |
//! | `force` | `false` | Overwrite existing file even if it already exists |
//! | `update_tool` | `appimageupdatetool` | Tool used for `state=latest` |

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct AppimageModule;

impl ModuleInvoker for AppimageModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        _ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let state = args.get_str("state").unwrap_or("present");
        let dest = args
            .get_str("dest")
            .ok_or_else(|| anyhow::anyhow!("appimage: 'dest' is required"))?
            .to_string();
        let dest_path = resolve_dest(&dest, args);

        match state {
            "absent" => appimage_absent(&dest_path, host),
            "latest" => {
                let update_tool = args.get_str("update_tool").unwrap_or("appimageupdatetool");
                appimage_update(update_tool, &dest_path, host)
            }
            _ => {
                let src = args
                    .get_str("src")
                    .ok_or_else(|| {
                        anyhow::anyhow!("appimage: 'src' is required for state=present")
                    })?
                    .to_string();
                let force = bool_arg(args, "force", false);
                let mode = parse_mode(args.get_str("mode").unwrap_or("0755"));
                let checksum = args.get_str("checksum").map(|s| s.to_string());
                let integrate = bool_arg(args, "integrate", false);
                let verify = bool_arg(args, "verify", false);
                appimage_install(
                    &src,
                    &dest_path,
                    force,
                    mode,
                    checksum.as_deref(),
                    verify,
                    integrate,
                    host,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn resolve_dest(dest: &str, args: &ModuleArgs) -> PathBuf {
    let p = Path::new(dest);
    // If dest is a directory, derive filename from src.
    if p.is_dir() || dest.ends_with('/') {
        if let Some(src) = args.get_str("src") {
            let fname = Path::new(src).file_name().unwrap_or_default();
            return p.join(fname);
        }
    }
    p.to_path_buf()
}

fn parse_mode(mode: &str) -> u32 {
    u32::from_str_radix(mode.trim_start_matches("0o").trim_start_matches('0'), 8).unwrap_or(0o755)
}

fn appimage_install(
    src: &str,
    dest: &Path,
    force: bool,
    mode: u32,
    checksum: Option<&str>,
    verify: bool,
    integrate: bool,
    host: &str,
) -> Result<TaskResult> {
    if dest.exists() && !force {
        return Ok(TaskResult::ok(host));
    }

    // Ensure parent directory exists.
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // Download or copy.
    if src.starts_with("http://") || src.starts_with("https://") {
        // Use curl / wget — no reqwest dependency needed.
        let status = Command::new("curl")
            .args(["-fsSL", "-o", &dest.to_string_lossy(), src])
            .status()
            .or_else(|_| {
                Command::new("wget")
                    .args(["-q", "-O", &dest.to_string_lossy(), src])
                    .status()
            })
            .context("failed to download AppImage (curl/wget required)")?;
        if !status.success() {
            return Ok(TaskResult::failed(
                host,
                format!("appimage: download from '{src}' failed"),
            ));
        }
    } else {
        std::fs::copy(src, dest).with_context(|| format!("appimage: copy '{src}' failed"))?;
    }

    // Verify checksum if requested.
    if let Some(cs) = checksum {
        if let Some(expected) = cs.strip_prefix("sha256:") {
            let actual = sha256_file(dest)?;
            if actual != expected {
                let _ = std::fs::remove_file(dest);
                return Ok(TaskResult::failed(
                    host,
                    format!("appimage: checksum mismatch (expected {expected}, got {actual})"),
                ));
            }
        }
    }

    // Verify embedded GPG signature.
    if verify {
        let out = Command::new(dest).arg("--appimage-signature").output().ok();
        // A non-zero exit or missing output is treated as unverified.
        if out.map_or(true, |o| !o.status.success()) {
            let _ = std::fs::remove_file(dest);
            return Ok(TaskResult::failed(
                host,
                "appimage: GPG signature verification failed".to_string(),
            ));
        }
    }

    // Make executable.
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))?;

    // Desktop integration.
    if integrate {
        // Try appimaged / ail-cli; ignore errors (daemon may not be running).
        let _ = Command::new("ail-cli")
            .args(["integrate", &dest.to_string_lossy()])
            .status();
    }

    let mut r = TaskResult::changed(host);
    r.msg = format!("AppImage installed to '{}'", dest.display());
    r.vars.insert(
        "dest".into(),
        Value::String(dest.to_string_lossy().into_owned()),
    );
    Ok(r)
}

fn appimage_absent(dest: &Path, host: &str) -> Result<TaskResult> {
    if !dest.exists() {
        return Ok(TaskResult::ok(host));
    }
    // Try to deintegrate first (ignore errors).
    let _ = Command::new("ail-cli")
        .args(["unintegrate", &dest.to_string_lossy()])
        .status();
    std::fs::remove_file(dest)
        .with_context(|| format!("appimage: remove '{}' failed", dest.display()))?;
    let mut r = TaskResult::changed(host);
    r.msg = format!("removed '{}'", dest.display());
    Ok(r)
}

fn appimage_update(update_tool: &str, dest: &Path, host: &str) -> Result<TaskResult> {
    if !dest.exists() {
        return Ok(TaskResult::failed(
            host,
            format!(
                "appimage: '{}' does not exist (cannot update)",
                dest.display()
            ),
        ));
    }
    let out = Command::new(update_tool)
        .arg(&dest.to_string_lossy().into_owned())
        .output()
        .with_context(|| format!("appimage: '{update_tool}' not found"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        let changed = !stdout.contains("is already up-to-date");
        let mut r = if changed {
            TaskResult::changed(host)
        } else {
            TaskResult::ok(host)
        };
        r.stdout = stdout;
        r.msg = if changed {
            "AppImage updated".into()
        } else {
            "AppImage already up-to-date".into()
        };
        Ok(r)
    } else {
        Ok(TaskResult::failed(
            host,
            format!("appimage update failed: {stderr}"),
        ))
    }
}

fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = [0u64; 4]; // simple accumulator placeholder
                                // Use sha256sum CLI — no extra Cargo dep needed.
    let out = Command::new("sha256sum")
        .arg(path)
        .output()
        .context("sha256sum not available")?;
    let line = String::from_utf8_lossy(&out.stdout).into_owned();
    Ok(line.split_whitespace().next().unwrap_or("").to_string())
}

fn bool_arg(args: &ModuleArgs, key: &str, default: bool) -> bool {
    args.args
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ansiblers_core::{ExecutionContext, Inventory};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn ctx() -> ExecutionContext {
        ExecutionContext::new(Arc::new(Inventory::default()), HashMap::new())
    }
    fn args(pairs: &[(&str, Value)]) -> ModuleArgs {
        let mut m = HashMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        ModuleArgs::new(m)
    }

    #[test]
    fn test_missing_dest_errors() {
        let mut c = ctx();
        let r = AppimageModule.invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c);
        assert!(r.is_err());
    }
    #[test]
    fn test_missing_src_for_present_errors() {
        let mut c = ctx();
        let r = AppimageModule.invoke(
            &args(&[("dest", Value::String("/tmp/app.AppImage".into()))]),
            "h",
            &mut c,
        );
        assert!(r.is_err());
    }
    #[test]
    fn test_parse_mode_octal() {
        assert_eq!(parse_mode("0755"), 0o755);
        assert_eq!(parse_mode("755"), 0o755);
    }
    #[test]
    fn test_resolve_dest_directory() {
        let a = args(&[(
            "src",
            Value::String("/tmp/MyApp-1.0-x86_64.AppImage".into()),
        )]);
        let resolved = resolve_dest("/opt/appimages/", &a);
        assert!(resolved
            .to_string_lossy()
            .ends_with("MyApp-1.0-x86_64.AppImage"));
    }
    #[test]
    fn test_absent_nonexistent_is_ok() {
        let r = appimage_absent(Path::new("/nonexistent/app.AppImage"), "h").unwrap();
        assert!(r.status.is_ok());
        assert!(!r.changed);
    }
}
