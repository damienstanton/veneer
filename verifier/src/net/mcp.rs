//! MCP adapters for the net tools: the same `Hub` code paths as the CLI.

use super::cli::task_json;
use super::error::NetError;
use super::hub::Hub;
use super::op::MemKind;
use crate::mcp::{MemoryArgs, TaskArgs};
use serde_json::{Value, json};

fn need<'a>(v: &'a Option<String>, what: &str) -> Result<&'a str, NetError> {
    v.as_deref().ok_or_else(|| NetError::Invalid(format!("{what} is required")))
}

pub fn task(a: TaskArgs) -> Result<Value, NetError> {
    let mut hub = Hub::load()?;
    let now = hub.replica.now();
    match a.action.as_str() {
        "list" => {
            let snap = hub.replica.snapshot()?;
            let tasks: Vec<Value> = snap
                .tasks
                .values()
                .filter(|t| a.all || !t.status_at(now).terminal())
                .map(|t| task_json(t, now))
                .collect();
            Ok(json!({"tasks": tasks}))
        }
        "show" => Ok(task_json(&hub.replica.task(need(&a.id, "id")?)?, now)),
        "new" => {
            let mut v = a.envelope.ok_or_else(|| NetError::Invalid("envelope is required".into()))?;
            if v.get("schema").is_none() {
                v["schema"] = json!(super::envelope::TASK_SCHEMA);
            }
            let t = serde_json::from_value(v).map_err(|e| NetError::Invalid(format!("envelope: {e}")))?;
            let id = hub.write(|r| r.create(t))?;
            Ok(json!({"id": id, "status": "queued"}))
        }
        "answer" => {
            let (id, text) = (need(&a.id, "id")?, need(&a.text, "text")?);
            hub.write(|r| r.answer(id, text)).map(|_| json!({"answered": id}))
        }
        "cancel" => {
            let (id, text) = (need(&a.id, "id")?, need(&a.text, "text")?);
            hub.write(|r| r.cancel(id, text)).map(|_| json!({"canceled": id}))
        }
        other => Err(NetError::Invalid(format!("action {other:?}: use list|show|new|answer|cancel"))),
    }
}

pub fn memory(a: MemoryArgs) -> Result<Value, NetError> {
    let mut hub = Hub::load()?;
    let kind = |k: Option<&str>| -> Result<Option<MemKind>, NetError> {
        k.map(|s| serde_json::from_value(json!(s)).map_err(|_| NetError::Invalid(format!("kind {s:?}"))))
            .transpose()
    };
    match a.action.as_str() {
        "put" => {
            let k = kind(a.kind.as_deref())?.ok_or_else(|| NetError::Invalid("kind is required".into()))?;
            let (scope, key, value) = (need(&a.scope, "scope")?, need(&a.key, "key")?, need(&a.value, "value")?);
            hub.write(|r| r.remember(scope, key, k, value)).map(|_| json!({"ok": true}))
        }
        "del" => {
            let (scope, key) = (need(&a.scope, "scope")?, need(&a.key, "key")?);
            hub.write(|r| r.forget(scope, key)).map(|_| json!({"ok": true}))
        }
        "get" => {
            let snap = hub.replica.snapshot()?;
            Ok(snap
                .memory
                .get(need(&a.scope, "scope")?, need(&a.key, "key")?)
                .map(|e| json!({"value": e.value, "kind": e.kind, "by": e.by, "at": e.at}))
                .unwrap_or(Value::Null))
        }
        "recall" => {
            let snap = hub.replica.snapshot()?;
            let k = kind(a.kind.as_deref())?;
            let entries: Vec<Value> = snap
                .memory
                .recall(a.repo.as_deref(), a.task.as_deref(), k, a.prefix.as_deref().unwrap_or(""))
                .into_iter()
                .map(|e| json!({"scope": e.scope, "key": e.key, "kind": e.kind, "value": e.value, "by": e.by}))
                .collect();
            Ok(json!({"entries": entries}))
        }
        other => Err(NetError::Invalid(format!("action {other:?}: use recall|get|put|del"))),
    }
}
