//! Integer sat/kvB policies. Upstream and the native table are already sat/kvB.

use crate::config::FeesConfig;
use std::collections::BTreeMap;

/// `FeeEstimatorConfig::default` `lnd_targets` in `blvm-node/src/config/fees.rs`.
/// Used when no upstream table names a target, so a native-only submit is not empty.
pub const NATIVE_TARGETS_WHEN_NO_UPSTREAM: &[u32] =
    &[1, 2, 3, 6, 12, 24, 48, 144, 504, 1008];

pub const MAX_QUOTES: usize = 64;

#[derive(Debug, Clone)]
pub struct FeeSource {
    pub id: String,
    pub height: u64,
    pub quotes: Vec<(u32, u64)>,
    pub lag_exempt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub without_upstream: bool,
    pub blend: Vec<(u32, u64)>,
    pub blend_min: Vec<(u32, u64)>,
}

pub fn targets_to_ask(upstream_targets: &[u32]) -> Vec<u32> {
    let mut targets = upstream_targets.to_vec();
    if targets.is_empty() {
        targets.extend_from_slice(NATIVE_TARGETS_WHEN_NO_UPSTREAM);
    }
    targets.sort_unstable();
    targets.dedup();
    targets.truncate(MAX_QUOTES);
    targets
}

/// `mergeFeeEstimates` on sat/kvB, after the priority order and the lag drop.
pub fn plan(config: &FeesConfig, native: FeeSource, upstream: &[FeeSource]) -> Planned {
    let healthy: Vec<&FeeSource> = upstream.iter().filter(|source| !source.quotes.is_empty()).collect();
    let without_upstream = healthy.is_empty();
    let mut best = native.height;
    for source in &healthy {
        best = best.max(source.height);
    }
    let mut fresh = vec![&native];
    for source in &healthy {
        if source.lag_exempt || best.saturating_sub(source.height) <= config.max_height_delta {
            fresh.push(*source);
        }
    }
    fresh.sort_by_key(|source| priority_index(&config.priority, &source.id));
    Planned {
        without_upstream,
        blend: merge_fee_estimates(&fresh),
        blend_min: prefix_min(&fresh),
    }
}

fn priority_index(priority: &[String], id: &str) -> (usize, String) {
    match priority.iter().position(|name| name == id) {
        Some(index) => (index, String::new()),
        None => (usize::MAX, id.to_string()),
    }
}

fn merge_fee_estimates(sources: &[&FeeSource]) -> Vec<(u32, u64)> {
    let mut merged: Vec<(u32, u64)> = Vec::new();
    for source in sources {
        let mut quotes = source.quotes.clone();
        quotes.sort_by_key(|(target, _)| *target);
        for (target, fee) in quotes {
            if fee == 0 {
                continue;
            }
            let longer = merged
                .iter()
                .map(|(accepted, _)| *accepted)
                .max()
                .is_some_and(|highest| target > highest);
            let cheaper = merged
                .iter()
                .map(|(_, accepted)| *accepted)
                .min()
                .is_some_and(|lowest| fee < lowest);
            if merged.is_empty() || (longer && cheaper) {
                merged.push((target, fee));
            }
        }
    }
    merged
}

fn prefix_min(sources: &[&FeeSource]) -> Vec<(u32, u64)> {
    let mut by_target: BTreeMap<u32, u64> = BTreeMap::new();
    for source in sources {
        for (target, fee) in &source.quotes {
            if *fee == 0 {
                continue;
            }
            by_target
                .entry(*target)
                .and_modify(|current| *current = (*current).min(*fee))
                .or_insert(*fee);
        }
    }
    let mut running = u64::MAX;
    let mut out = Vec::new();
    for (target, fee) in by_target {
        running = running.min(fee);
        out.push((target, running));
    }
    out
}
