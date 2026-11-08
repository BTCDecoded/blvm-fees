//! Port of LN-Zap `DataProviderManager.getData` / `mergeFeeEstimates`.
//!
//! Inputs are sat/vB, which is what that code merges. The published map is sat/kvB
//! after `toSatPerKvB`. Upstream tables are already sat/kvB, so the module does not
//! run this conversion on them.

#[derive(Clone)]
pub struct ProviderPoint {
    pub mempool: bool,
    pub order: usize,
    pub height: u64,
    pub hash: String,
    pub min_relay_sat_per_vb: f64,
    pub fees_sat_per_vb: Vec<(u32, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LnzapEstimates {
    pub height: u64,
    pub hash: String,
    pub fees_sat_per_kvb: Vec<(u32, u64)>,
    pub min_relay_sat_per_kvb: u64,
}

pub fn blend_lnzap(
    points: &[ProviderPoint],
    max_height_delta: u64,
    fee_multiplier: f64,
    fee_minimum_sat_per_vb: f64,
) -> Result<LnzapEstimates, String> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|left, right| {
        right
            .mempool
            .cmp(&left.mempool)
            .then(right.height.cmp(&left.height))
            .then(left.order.cmp(&right.order))
    });
    let rounded: Vec<ProviderPoint> = sorted
        .into_iter()
        .map(|mut point| {
            for (_, fee) in &mut point.fees_sat_per_vb {
                *fee = round_3(*fee);
            }
            point
        })
        .filter(|point| {
            !point.fees_sat_per_vb.is_empty()
                && point.min_relay_sat_per_vb.is_finite()
                && point.fees_sat_per_vb.iter().all(|(_, fee)| fee.is_finite())
        })
        .collect();
    if rounded.is_empty() {
        return Err("no data".to_string());
    }
    let best = rounded[0].height;
    let relevant: Vec<&ProviderPoint> = rounded
        .iter()
        .filter(|point| best.saturating_sub(point.height) <= max_height_delta)
        .collect();
    if relevant.is_empty() {
        return Err("no data".to_string());
    }
    let min_relay = relevant
        .iter()
        .map(|point| point.min_relay_sat_per_vb)
        .fold(f64::MIN, f64::max);
    let mut merged: Vec<(u32, f64)> = Vec::new();
    for point in &relevant {
        let mut keys: Vec<(u32, f64)> = point
            .fees_sat_per_vb
            .iter()
            .copied()
            .filter(|(_, fee)| *fee >= fee_minimum_sat_per_vb && *fee >= min_relay)
            .collect();
        keys.sort_by_key(|(target, _)| *target);
        for (target, fee) in keys {
            let longer = merged
                .iter()
                .map(|(accepted, _)| *accepted)
                .max()
                .is_some_and(|highest| target > highest);
            let cheaper = merged
                .iter()
                .map(|(_, accepted)| *accepted)
                .reduce(f64::min)
                .is_some_and(|lowest| fee < lowest);
            if merged.is_empty() || (longer && cheaper) {
                merged.push((target, fee));
            }
        }
    }
    let mut fees: Vec<(u32, u64)> = merged
        .into_iter()
        .map(|(target, fee)| (target, to_sat_per_kvb(fee, fee_multiplier)))
        .collect();
    let mut relay = to_sat_per_kvb(min_relay, fee_multiplier);
    let configured = to_sat_per_kvb(fee_minimum_sat_per_vb, 1.0);
    if fees.is_empty() {
        fees.push((1, configured.max(relay)));
    }
    relay = relay.max(configured);
    Ok(LnzapEstimates {
        height: relevant[0].height,
        hash: relevant[0].hash.clone(),
        fees_sat_per_kvb: fees,
        min_relay_sat_per_kvb: relay,
    })
}

fn round_3(value: f64) -> f64 {
    ((value + f64::EPSILON) * 1000.0).round() / 1000.0
}

/// `Math.ceil(Number((satPerVb * 1000 * multiplier).toFixed(3)))`.
fn to_sat_per_kvb(sat_per_vb: f64, multiplier: f64) -> u64 {
    let scaled = sat_per_vb * 1000.0 * multiplier;
    let fixed: f64 = format!("{scaled:.3}").parse().unwrap_or(scaled);
    fixed.ceil() as u64
}
