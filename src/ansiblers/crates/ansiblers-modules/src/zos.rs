//! `zos` module — manage IBM z/OS system resources via z/OSMF REST API,
//! TSO/E commands, ISPF services, and UNIX System Services (USS) operations.
//!
//! This module provides a unified interface for common z/OS automation tasks:
//!
//! ## Operations (`operation` parameter)
//!
//! | `operation` | Description |
//! |-------------|-------------|
//! | `job_submit` | Submit a JCL job to the internal reader |
//! | `job_status` | Query the status of a job by name or ID |
//! | `job_cancel` | Cancel a running job |
//! | `job_purge` | Purge completed job output |
//! | `dataset_create` | Allocate a z/OS dataset (sequential, PDS, PDSE) |
//! | `dataset_delete` | Delete a z/OS dataset or member |
//! | `dataset_copy` | Copy a dataset or member |
//! | `dataset_fetch` | Read a dataset and store contents as a fact |
//! | `dataset_write` | Write content to a dataset |
//! | `member_list` | List PDS/PDSE members |
//! | `command` | Submit a TSO/E or operator command via z/OSMF |
//! | `uss_command` | Run a shell command in USS |
//! | `sysvar` | Retrieve a z/OS system variable |
//!
//! ## Connectivity parameters
//!
//! | Parameter | Default | Description |
//! |-----------|---------|-------------|
//! | `zosmf_host` | — | z/OSMF hostname or IP address |
//! | `zosmf_port` | `443` | z/OSMF HTTPS port |
//! | `zosmf_user` | — | z/OSMF username |
//! | `zosmf_password` | — | z/OSMF password (use `no_log: true`) |
//! | `zosmf_certificate` | — | Path to client certificate |
//! | `verify_ssl` | `true` | Verify SSL certificate |
//! | `base_url` | — | Full z/OSMF base URL (overrides host/port) |
//!
//! ## Operation-specific parameters
//!
//! | Parameter | Description |
//! |-----------|-------------|
//! | `operation` | Required — operation name from the table above |
//! | `jcl` | JCL content to submit (for `job_submit`) |
//! | `jcl_file` | Path to a JCL file (for `job_submit`) |
//! | `job_name` | Job name filter for `job_status`/`job_cancel`/`job_purge` |
//! | `job_id` | Specific job ID (e.g. `JOB00001`) |
//! | `dataset` | Dataset name (e.g. `USER.DATA.SEQ`) |
//! | `member` | PDS member name |
//! | `content` | Data content to write |
//! | `content_file` | Local file whose content to write to a dataset |
//! | `dsorg` | Dataset organisation: `PS`, `PO`, `PDSE` |
//! | `recfm` | Record format: `FB`, `VB`, `U`, etc. |
//! | `lrecl` | Logical record length |
//! | `blksize` | Block size |
//! | `primary` | Primary allocation (tracks or cylinders) |
//! | `secondary` | Secondary allocation |
//! | `unit` | Unit type (`SYSDA`, `3390`) |
//! | `tso_command` | TSO/E command string |
//! | `uss_path` | USS path for `uss_command` |
//! | `timeout` | HTTP request timeout in seconds |

use std::collections::HashMap;
use std::process::Command;

use ansiblers_core::{ExecutionContext, TaskResult, Value};
use anyhow::{Context as _, Result};

use crate::registry::{ModuleArgs, ModuleInvoker};

pub struct ZosModule;

impl ModuleInvoker for ZosModule {
    fn invoke(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let operation = args
            .get_str("operation")
            .ok_or_else(|| anyhow::anyhow!("zos: 'operation' is required"))?;

        let client = ZosmfClient::from_args(args)?;

        match operation {
            "job_submit" => client.job_submit(args, host),
            "job_status" => client.job_status(args, host, ctx),
            "job_cancel" => client.job_cancel(args, host),
            "job_purge" => client.job_purge(args, host),
            "dataset_create" => client.dataset_create(args, host),
            "dataset_delete" => client.dataset_delete(args, host),
            "dataset_copy" => client.dataset_copy(args, host),
            "dataset_fetch" => client.dataset_fetch(args, host, ctx),
            "dataset_write" => client.dataset_write(args, host),
            "member_list" => client.member_list(args, host, ctx),
            "command" => client.tso_command(args, host, ctx),
            "uss_command" => client.uss_command(args, host),
            "sysvar" => client.sysvar(args, host, ctx),
            other => Ok(TaskResult::failed(
                host,
                format!("zos: unknown operation '{other}'"),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// z/OSMF client (builds curl / requests-style CLI calls)
// ---------------------------------------------------------------------------

struct ZosmfClient {
    base_url: String,
    user: String,
    password: String,
    verify_ssl: bool,
    timeout: u64,
}

impl ZosmfClient {
    fn from_args(args: &ModuleArgs) -> Result<Self> {
        let host = args.get_str("zosmf_host");
        let port = args
            .args
            .get("zosmf_port")
            .and_then(|v| v.as_u64())
            .unwrap_or(443);
        let base_url = args
            .get_str("base_url")
            .map(|s| s.to_string())
            .or_else(|| host.map(|h| format!("https://{h}:{port}/zosmf")))
            .ok_or_else(|| anyhow::anyhow!("zos: 'zosmf_host' or 'base_url' is required"))?;

        let user = args
            .get_str("zosmf_user")
            .ok_or_else(|| anyhow::anyhow!("zos: 'zosmf_user' is required"))?
            .to_string();
        let password = args
            .get_str("zosmf_password")
            .ok_or_else(|| anyhow::anyhow!("zos: 'zosmf_password' is required"))?
            .to_string();

        Ok(Self {
            base_url,
            user,
            password,
            verify_ssl: args
                .args
                .get("verify_ssl")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            timeout: args
                .args
                .get("timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(30),
        })
    }

    fn curl_base(&self) -> Command {
        let mut cmd = Command::new("curl");
        cmd.args(["-s", "-S"]);
        cmd.args(["-u", &format!("{}:{}", self.user, self.password)]);
        cmd.args(["--max-time", &self.timeout.to_string()]);
        if !self.verify_ssl {
            cmd.arg("-k");
        }
        cmd.args(["-H", "Content-Type: application/json"]);
        cmd.args(["-H", "X-CSRF-ZOSMF-HEADER: true"]);
        cmd
    }

    fn get(&self, path: &str) -> Command {
        let mut cmd = self.curl_base();
        cmd.arg(format!("{}{}", self.base_url, path));
        cmd
    }

    fn post_json(&self, path: &str, body: &str) -> Command {
        let mut cmd = self.curl_base();
        cmd.args(["-X", "POST"]);
        cmd.args(["-d", body]);
        cmd.arg(format!("{}{}", self.base_url, path));
        cmd
    }

    fn delete(&self, path: &str) -> Command {
        let mut cmd = self.curl_base();
        cmd.args(["-X", "DELETE"]);
        cmd.arg(format!("{}{}", self.base_url, path));
        cmd
    }

    fn put_data(&self, path: &str, data: &str) -> Command {
        let mut cmd = self.curl_base();
        cmd.args(["-X", "PUT", "-d", data]);
        cmd.arg(format!("{}{}", self.base_url, path));
        cmd
    }

    fn run_and_parse(
        &self,
        mut cmd: Command,
        host: &str,
        changed: bool,
        msg: &str,
    ) -> Result<TaskResult> {
        let out = cmd.output().context("curl (z/OSMF)")?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if out.status.success() {
            let mut r = if changed {
                TaskResult::changed(host)
            } else {
                TaskResult::ok(host)
            };
            r.stdout = stdout.clone();
            r.msg = msg.to_string();
            // Try to parse JSON response into vars.
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&stdout) {
                r.vars.insert("response".into(), json.into());
            }
            Ok(r)
        } else {
            Ok(TaskResult::failed(
                host,
                format!("z/OSMF request failed: {stderr}{stdout}"),
            ))
        }
    }

    // -----------------------------------------------------------------------
    // Job operations
    // -----------------------------------------------------------------------

    fn job_submit(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let jcl = if let Some(j) = args.get_str("jcl") {
            j.to_string()
        } else if let Some(f) = args.get_str("jcl_file") {
            std::fs::read_to_string(f).with_context(|| format!("cannot read JCL file '{f}'"))?
        } else {
            return Ok(TaskResult::failed(
                host,
                "zos: 'jcl' or 'jcl_file' is required for job_submit".to_string(),
            ));
        };

        let body = serde_json::json!({ "file": "//DD:INPUT" }).to_string();
        let cmd = self.post_json(
            "/restjobs/jobs",
            &serde_json::json!({ "jcl": jcl }).to_string(),
        );
        self.run_and_parse(cmd, host, true, "job submitted")
    }

    fn job_status(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let path = if let Some(id) = args.get_str("job_id") {
            format!("/restjobs/jobs?jobid={id}")
        } else if let Some(name) = args.get_str("job_name") {
            format!("/restjobs/jobs?prefix={name}")
        } else {
            "/restjobs/jobs".to_string()
        };
        let mut r = self.run_and_parse(self.get(&path), host, false, "job status retrieved")?;
        if let Some(resp) = r.vars.get("response") {
            ctx.set_fact(host, "zos_job_status".into(), resp.clone());
        }
        Ok(r)
    }

    fn job_cancel(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let id = args
            .get_str("job_id")
            .ok_or_else(|| anyhow::anyhow!("zos: 'job_id' is required for job_cancel"))?;
        let body = serde_json::json!({ "request": "cancel" }).to_string();
        self.run_and_parse(
            self.post_json(&format!("/restjobs/jobs/{id}/operations"), &body),
            host,
            true,
            "job cancelled",
        )
    }

    fn job_purge(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let id = args
            .get_str("job_id")
            .ok_or_else(|| anyhow::anyhow!("zos: 'job_id' is required for job_purge"))?;
        self.run_and_parse(
            self.delete(&format!("/restjobs/jobs/{id}")),
            host,
            true,
            "job purged",
        )
    }

    // -----------------------------------------------------------------------
    // Dataset operations
    // -----------------------------------------------------------------------

    fn dataset_create(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let ds = args
            .get_str("dataset")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dataset' is required for dataset_create"))?;
        let dsorg = args.get_str("dsorg").unwrap_or("PS");
        let recfm = args.get_str("recfm").unwrap_or("FB");
        let lrecl = args
            .args
            .get("lrecl")
            .and_then(|v| v.as_u64())
            .unwrap_or(80);
        let primary = args
            .args
            .get("primary")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);
        let secondary = args
            .args
            .get("secondary")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let unit = args.get_str("unit").unwrap_or("SYSDA");

        let body = serde_json::json!({
            "dsorg": dsorg,
            "recfm": recfm,
            "lrecl": lrecl,
            "primary": primary,
            "secondary": secondary,
            "unit": unit,
            "alcunit": "TRK"
        })
        .to_string();
        let cmd = self.post_json(&format!("/zosmf/restfiles/ds/{}", ds.to_uppercase()), &body);
        self.run_and_parse(cmd, host, true, &format!("dataset '{ds}' created"))
    }

    fn dataset_delete(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let ds = args
            .get_str("dataset")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dataset' is required for dataset_delete"))?;
        let path = if let Some(mem) = args.get_str("member") {
            format!(
                "/zosmf/restfiles/ds/{}({})",
                ds.to_uppercase(),
                mem.to_uppercase()
            )
        } else {
            format!("/zosmf/restfiles/ds/{}", ds.to_uppercase())
        };
        self.run_and_parse(
            self.delete(&path),
            host,
            true,
            &format!("dataset '{ds}' deleted"),
        )
    }

    fn dataset_copy(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let src = args.get_str("dataset").ok_or_else(|| {
            anyhow::anyhow!("zos: 'dataset' (source) is required for dataset_copy")
        })?;
        let dest = args
            .get_str("dest")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dest' is required for dataset_copy"))?;
        let body = serde_json::json!({
            "request": "copy",
            "from-dataset": { "dsn": src.to_uppercase() }
        })
        .to_string();
        let cmd = self.post_json(
            &format!("/zosmf/restfiles/ds/{}", dest.to_uppercase()),
            &body,
        );
        self.run_and_parse(
            cmd,
            host,
            true,
            &format!("dataset '{src}' copied to '{dest}'"),
        )
    }

    fn dataset_fetch(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let ds = args
            .get_str("dataset")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dataset' is required for dataset_fetch"))?;
        let path = if let Some(mem) = args.get_str("member") {
            format!(
                "/zosmf/restfiles/ds/{}({})",
                ds.to_uppercase(),
                mem.to_uppercase()
            )
        } else {
            format!("/zosmf/restfiles/ds/{}", ds.to_uppercase())
        };
        let mut r = self.run_and_parse(self.get(&path), host, false, &format!("fetched '{ds}'"))?;
        ctx.set_fact(
            host,
            "zos_dataset_content".into(),
            Value::String(r.stdout.clone()),
        );
        r.vars
            .insert("content".into(), Value::String(r.stdout.clone()));
        Ok(r)
    }

    fn dataset_write(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let ds = args
            .get_str("dataset")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dataset' is required for dataset_write"))?;
        let content = if let Some(c) = args.get_str("content") {
            c.to_string()
        } else if let Some(f) = args.get_str("content_file") {
            std::fs::read_to_string(f)?
        } else {
            return Ok(TaskResult::failed(
                host,
                "zos: 'content' or 'content_file' required for dataset_write".to_string(),
            ));
        };
        let path = if let Some(mem) = args.get_str("member") {
            format!(
                "/zosmf/restfiles/ds/{}({})",
                ds.to_uppercase(),
                mem.to_uppercase()
            )
        } else {
            format!("/zosmf/restfiles/ds/{}", ds.to_uppercase())
        };
        self.run_and_parse(
            self.put_data(&path, &content),
            host,
            true,
            &format!("wrote to '{ds}'"),
        )
    }

    fn member_list(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let ds = args
            .get_str("dataset")
            .ok_or_else(|| anyhow::anyhow!("zos: 'dataset' is required for member_list"))?;
        let path = format!("/zosmf/restfiles/ds/{}/member", ds.to_uppercase());
        let mut r = self.run_and_parse(
            self.get(&path),
            host,
            false,
            &format!("listed members of '{ds}'"),
        )?;
        // Parse member list from JSON { "items": [{"member": "NAME"}, ...] }
        if let Some(resp) = r.vars.get("response") {
            if let Some(arr) = resp.get("items").and_then(|v| v.as_array()) {
                let members: Vec<Value> = arr
                    .iter()
                    .filter_map(|v| {
                        v.get("member")
                            .and_then(|m| m.as_str())
                            .map(|s| Value::String(s.to_string()))
                    })
                    .collect();
                ctx.set_fact(host, "zos_members".into(), Value::Array(members.clone()));
                r.vars.insert("members".into(), Value::Array(members));
            }
        }
        Ok(r)
    }

    // -----------------------------------------------------------------------
    // Command operations
    // -----------------------------------------------------------------------

    fn tso_command(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let cmd_str = args.get_str("tso_command").ok_or_else(|| {
            anyhow::anyhow!("zos: 'tso_command' is required for operation=command")
        })?;
        let body = serde_json::json!({ "cmd": cmd_str }).to_string();
        let mut r = self.run_and_parse(
            self.post_json("/zosmf/restconsoles/consoles/defcn", &body),
            host,
            true,
            &format!("ran TSO command: {cmd_str}"),
        )?;
        if let Some(resp) = r.vars.get("response") {
            ctx.set_fact(host, "zos_command_response".into(), resp.clone());
        }
        Ok(r)
    }

    fn uss_command(&self, args: &ModuleArgs, host: &str) -> Result<TaskResult> {
        let cmd_str = args.get_str("command").ok_or_else(|| {
            anyhow::anyhow!("zos: 'command' is required for operation=uss_command")
        })?;
        let uss_path = args.get_str("uss_path").unwrap_or("/bin/sh");
        let body = serde_json::json!({
            "cmd": cmd_str,
            "cwd": uss_path
        })
        .to_string();
        self.run_and_parse(
            self.post_json("/zosmf/restfiles/process", &body),
            host,
            true,
            &format!("ran USS command: {cmd_str}"),
        )
    }

    fn sysvar(
        &self,
        args: &ModuleArgs,
        host: &str,
        ctx: &mut ExecutionContext,
    ) -> Result<TaskResult> {
        let varname = args
            .get_str("name")
            .ok_or_else(|| anyhow::anyhow!("zos: 'name' is required for operation=sysvar"))?;
        let path = format!("/zosmf/variables/rest/1.0/systems/-/names/{varname}");
        let mut r = self.run_and_parse(
            self.get(&path),
            host,
            false,
            &format!("retrieved sysvar '{varname}'"),
        )?;
        if let Some(resp) = r.vars.get("response") {
            ctx.set_fact(
                host,
                format!("zos_sysvar_{}", varname.to_lowercase()),
                resp.clone(),
            );
        }
        Ok(r)
    }
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
    fn test_missing_operation_errors() {
        let mut c = ctx();
        let r = ZosModule.invoke(&ModuleArgs::new(HashMap::new()), "h", &mut c);
        assert!(r.is_err());
    }
    #[test]
    fn test_unknown_operation_fails() {
        let mut c = ctx();
        let a = args(&[
            ("operation", Value::String("frobnicate".into())),
            ("zosmf_host", Value::String("zos.example.com".into())),
            ("zosmf_user", Value::String("ibmuser".into())),
            ("zosmf_password", Value::String("secret".into())),
        ]);
        let r = ZosModule.invoke(&a, "h", &mut c).unwrap();
        assert!(r.status.is_failed());
        assert!(r.msg.contains("unknown operation"));
    }
    #[test]
    fn test_missing_zosmf_host_errors() {
        let mut c = ctx();
        let a = args(&[("operation", Value::String("job_status".into()))]);
        let r = ZosModule.invoke(&a, "h", &mut c);
        assert!(r.is_err());
    }
    #[test]
    fn test_job_submit_missing_jcl_errors() {
        let client = ZosmfClient {
            base_url: "https://zos.example.com:443/zosmf".into(),
            user: "ibmuser".into(),
            password: "secret".into(),
            verify_ssl: true,
            timeout: 30,
        };
        let a = ModuleArgs::new(HashMap::new());
        let mut c = ctx();
        let r = client.job_submit(&a, "h").unwrap();
        assert!(r.status.is_failed());
    }
    #[test]
    fn test_dataset_create_missing_dataset_errors() {
        let client = ZosmfClient {
            base_url: "https://zos.example.com:443/zosmf".into(),
            user: "ibmuser".into(),
            password: "secret".into(),
            verify_ssl: true,
            timeout: 30,
        };
        let a = ModuleArgs::new(HashMap::new());
        let r = client.dataset_create(&a, "h");
        assert!(r.is_err());
    }
    #[test]
    fn test_verify_ssl_default_true() {
        let a = ModuleArgs::new(HashMap::new());
        assert!(bool_arg(&a, "verify_ssl", true));
    }
    #[test]
    fn test_dataset_write_missing_content_fails() {
        let client = ZosmfClient {
            base_url: "https://zos.example.com:443/zosmf".into(),
            user: "u".into(),
            password: "p".into(),
            verify_ssl: true,
            timeout: 10,
        };
        let a = args(&[("dataset", Value::String("USER.DATA".into()))]);
        let r = client.dataset_write(&a, "h").unwrap();
        assert!(r.status.is_failed());
    }
}
