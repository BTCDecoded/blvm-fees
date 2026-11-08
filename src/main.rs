//! blvm-fees. Registers `blend` and `blend-min` and submits on each fresh native table.

use anyhow::Result;
use async_trait::async_trait;
use blvm_fees::{FeeSource, FeesConfig, plan, targets_to_ask};
use blvm_node::module::inter_module::api::ModuleAPI;
use blvm_node::module::ipc::protocol::{CliSpec, ExternalFeeTableWire, FeeEstimatorSpecWire};
use blvm_node::module::traits::{EventType, ModuleError, NodeAPI};
use blvm_sdk::module::runner::run_module_with_setup_and_api;
use blvm_sdk::module::{ModuleBootstrap, ModuleDb};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tracing::warn;

const MODULE_NAME: &str = "blvm-fees";

struct NoApi;

#[async_trait]
impl ModuleAPI for NoApi {
    async fn handle_request(
        &self,
        method: &str,
        _params: &[u8],
        _caller_module_id: &str,
    ) -> Result<Vec<u8>, ModuleError> {
        Err(ModuleError::OperationError(format!(
            "blvm-fees has no method {method}"
        )))
    }

    fn list_methods(&self) -> Vec<String> {
        Vec::new()
    }

    fn api_version(&self) -> u32 {
        1
    }
}

fn specs() -> Vec<FeeEstimatorSpecWire> {
    vec![
        FeeEstimatorSpecWire {
            name: "blend".to_string(),
            description: "LN-Zap mergeFeeEstimates".to_string(),
            modes: vec!["conservative".to_string(), "economical".to_string()],
            default_merge: "replace".to_string(),
            auto_priority: None,
        },
        FeeEstimatorSpecWire {
            name: "blend-min".to_string(),
            description: "Per-target minimum, then a prefix-min".to_string(),
            modes: vec!["conservative".to_string(), "economical".to_string()],
            default_merge: "replace".to_string(),
            auto_priority: None,
        },
    ]
}

async fn submit_cycle(node: &dyn NodeAPI, module_id: &str, config: &FeesConfig) {
    let timeout = Duration::from_millis(config.request_timeout_ms);
    let modules = match node.discover_modules().await {
        Ok(modules) => modules,
        Err(err) => {
            warn!("module discovery failed: {err}");
            Vec::new()
        }
    };
    let mut upstream = Vec::new();
    let mut upstream_targets = Vec::new();
    for module in modules {
        if module.module_name == MODULE_NAME {
            continue;
        }
        let call = node.call_module(Some(&module.module_id), "sources", b"{}".to_vec());
        let Ok(Ok(body)) = tokio::time::timeout(timeout, call).await else {
            continue;
        };
        let Ok(rows) = serde_json::from_slice::<Vec<Value>>(&body) else {
            continue;
        };
        for row in rows {
            let Some(id) = row.get("id").and_then(Value::as_str) else {
                continue;
            };
            if row.get("healthy").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let params = serde_json::json!({"source": id}).to_string().into_bytes();
            let call = node.call_module(Some(&module.module_id), "fee_table", params);
            let Ok(Ok(body)) = tokio::time::timeout(timeout, call).await else {
                continue;
            };
            let Ok(table) = serde_json::from_slice::<Value>(&body) else {
                continue;
            };
            let Some(height) = table.get("tip_height").and_then(Value::as_u64) else {
                continue;
            };
            let quotes = quotes_from(&table);
            upstream_targets.extend(quotes.iter().map(|(target, _)| *target));
            upstream.push(FeeSource {
                id: id.to_string(),
                height,
                quotes,
                lag_exempt: false,
            });
        }
    }
    let targets = targets_to_ask(&upstream_targets);
    let Ok(native) = node
        .get_fee_table("combined", "conservative", &targets)
        .await
    else {
        return;
    };
    let native_source = FeeSource {
        id: "native".to_string(),
        height: native.tip_height,
        quotes: native
            .quotes
            .iter()
            .filter_map(|(target, quote)| quote.as_ref().map(|quote| (*target, quote.sat_per_kvb)))
            .collect(),
        lag_exempt: true,
    };
    if native_source.quotes.is_empty() {
        return;
    }
    let planned = plan(config, native_source, &upstream);
    if planned.without_upstream {
        warn!("blvm-fees is running without upstream data");
    }
    let tables = [("blend", planned.blend), ("blend-min", planned.blend_min)];
    for (index, (name, quotes)) in tables.into_iter().enumerate() {
        if quotes.is_empty() {
            continue;
        }
        if index > 0 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let table = ExternalFeeTableWire {
            estimator: name.to_string(),
            tip_hash: native.tip_hash,
            upstream_height: native.tip_height,
            mode: None,
            quotes,
        };
        if let Err(err) = node.submit_fee_table(module_id, table).await {
            warn!(estimator = name, "submit failed: {err}");
        }
    }
}

fn quotes_from(table: &Value) -> Vec<(u32, u64)> {
    let Some(rows) = table.get("quotes").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut quotes = Vec::new();
    for row in rows {
        let Some(pair) = row.as_array() else {
            continue;
        };
        let (Some(target), Some(rate)) = (
            pair.first().and_then(Value::as_u64),
            pair.get(1).and_then(Value::as_u64),
        ) else {
            continue;
        };
        if target == 0 || target > u64::from(u32::MAX) || rate == 0 {
            continue;
        }
        quotes.push((target as u32, rate));
    }
    quotes
}

#[tokio::main]
async fn main() -> Result<()> {
    let bootstrap = ModuleBootstrap::init_module(MODULE_NAME);
    let db = ModuleDb::open_or_temp(&bootstrap.data_dir, MODULE_NAME)?;
    let data_dir = bootstrap.data_dir.clone();
    let module_id = bootstrap.module_id.clone();
    let trigger = Arc::new(Notify::new());
    let event_trigger = Arc::clone(&trigger);
    let setup = {
        let trigger = Arc::clone(&trigger);
        let module_id = module_id.clone();
        move |node_api: Arc<dyn NodeAPI>,
              _db: Arc<dyn blvm_node::storage::database::Database>,
              dir: &std::path::Path| {
            let dir = dir.to_path_buf();
            let trigger = Arc::clone(&trigger);
            let module_id = module_id.clone();
            async move {
                let mut config = FeesConfig::load(dir.join("config.toml"))
                    .map_err(|err| ModuleError::Other(err.to_string()))?;
                config
                    .apply_scalar_overrides(
                        std::env::var("MODULE_CONFIG_MAX_HEIGHT_DELTA")
                            .ok()
                            .as_deref(),
                        std::env::var("MODULE_CONFIG_REQUEST_TIMEOUT_MS")
                            .ok()
                            .as_deref(),
                        std::env::var("MODULE_CONFIG_SUBMIT_INTERVAL_SECS")
                            .ok()
                            .as_deref(),
                    )
                    .map_err(|err| ModuleError::Other(err.to_string()))?;
                node_api
                    .register_fee_estimators(&module_id, specs())
                    .await?;
                let node = Arc::clone(&node_api);
                let interval = config.submit_interval_secs.max(1);
                tokio::spawn(async move {
                    loop {
                        submit_cycle(node.as_ref(), &module_id, &config).await;
                        tokio::select! {
                            _ = trigger.notified() => {}
                            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
                        }
                    }
                });
                Ok(((), (), Arc::new(NoApi) as Arc<dyn ModuleAPI>))
            }
        }
    };
    run_module_with_setup_and_api(
        &bootstrap.socket_path,
        &bootstrap.module_id,
        MODULE_NAME,
        env!("CARGO_PKG_VERSION"),
        CliSpec {
            version: 1,
            name: MODULE_NAME.to_string(),
            about: Some("blend and blend-min fee estimators".to_string()),
            subcommands: Vec::new(),
        },
        &[],
        vec![EventType::FeeTableReady],
        |_invocation, _ctx, _module, _cli| {
            blvm_node::module::ipc::protocol::InvocationResultMessage {
                correlation_id: 0,
                success: false,
                payload: None,
                error: Some("no cli".to_string()),
            }
        },
        move |event, _module, _ctx| {
            let trigger = Arc::clone(&event_trigger);
            async move {
                if event.event_type == EventType::FeeTableReady {
                    trigger.notify_one();
                }
                Ok(())
            }
        },
        setup,
        db.as_db(),
        &data_dir,
    )
    .await
    .map_err(|err| anyhow::anyhow!("{err}"))?;
    Ok(())
}
