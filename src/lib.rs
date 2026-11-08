//! `blend` and `blend-min`. No calls to mempool.space or blockstream.

mod blend;
mod config;
mod lnzap;

pub use blend::{plan, targets_to_ask, FeeSource, Planned, NATIVE_TARGETS_WHEN_NO_UPSTREAM};
pub use config::FeesConfig;
pub use lnzap::{blend_lnzap, LnzapEstimates, ProviderPoint};

#[cfg(test)]
mod tests {
    use super::*;

    fn point(
        order: usize,
        height: u64,
        hash: &str,
        fees: &[(u32, f64)],
    ) -> ProviderPoint {
        ProviderPoint {
            mempool: false,
            order,
            height,
            hash: hash.to_string(),
            min_relay_sat_per_vb: 1.0,
            fees_sat_per_vb: fees.to_vec(),
        }
    }

    #[test]
    fn blend_matches_the_lnzap_merge_fixture() {
        // test/DataProviderManager-merge.test.ts
        // maxHeightDelta 2, feeMultiplier 2, feeMinimum 2.
        let points = vec![
            point(0, 1001, "hash1001", &[(1, 1.0), (2, 1.0), (3, 1.0)]),
            point(1, 998, "hash998", &[(1, 20.0), (10, 1.0)]),
            point(2, 1000, "hash1000", &[(1, 30.0), (2, 20.0)]),
            point(
                3,
                999,
                "hash999",
                &[
                    (1, 25.0),
                    (2, 15.0),
                    (3, 5.0),
                    (5, 3.564999999999998),
                    (6, 3.564999999999998),
                    (7, 3.564999999999998),
                    (8, 3.564999999999998),
                    (9, 3.564999999999998),
                ],
            ),
        ];
        let merged = blend_lnzap(&points, 2, 2.0, 2.0).unwrap();
        assert_eq!(merged.height, 1001);
        assert_eq!(merged.hash, "hash1001");
        assert_eq!(
            merged.fees_sat_per_kvb,
            vec![(1, 60_000), (2, 40_000), (3, 10_000), (5, 7_130)]
        );
        assert_eq!(merged.min_relay_sat_per_kvb, 2_000);
    }

    #[test]
    fn blend_matches_the_lnzap_minimum_fixtures() {
        let one = |fees: &[(u32, f64)], relay: f64| ProviderPoint {
            mempool: false,
            order: 0,
            height: 1001,
            hash: "hash1001".to_string(),
            min_relay_sat_per_vb: relay,
            fees_sat_per_vb: fees.to_vec(),
        };
        let below_min = blend_lnzap(&[one(&[(1, 3.0), (2, 2.0), (3, 1.0)], 1.0)], 1, 1.0, 2.0).unwrap();
        assert_eq!(below_min.fees_sat_per_kvb, vec![(1, 3_000), (2, 2_000)]);
        assert_eq!(below_min.min_relay_sat_per_kvb, 2_000);

        let below_relay = blend_lnzap(&[one(&[(1, 3.0), (2, 2.0), (3, 1.0)], 2.0)], 1, 1.0, 1.0).unwrap();
        assert_eq!(below_relay.fees_sat_per_kvb, vec![(1, 3_000), (2, 2_000)]);
        assert_eq!(below_relay.min_relay_sat_per_kvb, 2_000);

        let fallback = blend_lnzap(&[one(&[(1, 1.0), (2, 1.0), (3, 1.0)], 1.0)], 1, 1.0, 2.0).unwrap();
        assert_eq!(fallback.fees_sat_per_kvb, vec![(1, 2_000)]);
        assert_eq!(fallback.min_relay_sat_per_kvb, 2_000);

        let fractional = blend_lnzap(&[one(&[(1, 3.0), (2, 2.0)], 1.013)], 1, 1.0, 1.0).unwrap();
        assert_eq!(fractional.min_relay_sat_per_kvb, 1_013);
        assert_eq!(fractional.fees_sat_per_kvb, vec![(1, 3_000), (2, 2_000)]);

        let scaled = blend_lnzap(&[one(&[(1, 3.333), (2, 2.5)], 1.013)], 1, 1.5, 1.0).unwrap();
        assert_eq!(scaled.min_relay_sat_per_kvb, 1_520);
        assert_eq!(scaled.fees_sat_per_kvb, vec![(1, 5_000), (2, 3_750)]);

        let relay_fallback = blend_lnzap(&[one(&[(1, 1.0), (2, 1.0)], 1.013)], 1, 1.0, 1.0).unwrap();
        assert_eq!(relay_fallback.fees_sat_per_kvb, vec![(1, 1_013)]);
        assert_eq!(relay_fallback.min_relay_sat_per_kvb, 1_013);

        let inflated = blend_lnzap(&[one(&[(1, 3.0), (2, 2.007)], 2.007)], 1, 1.0, 1.0).unwrap();
        assert_eq!(inflated.min_relay_sat_per_kvb, 2_007);
        assert_eq!(inflated.fees_sat_per_kvb[1].1, 2_007);

        for rate in [1.29, 1.3, 1.33, 1.36] {
            let matched = blend_lnzap(&[one(&[(1, rate)], rate)], 1, 1.1, 1.0).unwrap();
            assert_eq!(matched.fees_sat_per_kvb[0].1, matched.min_relay_sat_per_kvb);
        }
    }

    fn source(id: &str, height: u64, quotes: &[(u32, u64)]) -> FeeSource {
        FeeSource {
            id: id.to_string(),
            height,
            quotes: quotes.to_vec(),
            lag_exempt: false,
        }
    }

    #[test]
    fn blend_min_is_the_per_target_minimum_then_a_prefix_min() {
        let mut native = source("native", 100, &[(1, 100), (3, 80)]);
        native.lag_exempt = true;
        let upstream = [
            source("mempool-space", 100, &[(1, 90), (3, 70), (6, 95)]),
            source("blockstream", 96, &[(1, 1)]),
        ];
        let planned = plan(&FeesConfig::default(), native, &upstream);
        assert!(!planned.without_upstream);
        assert_eq!(planned.blend_min, vec![(1, 90), (3, 70), (6, 70)]);
        assert_eq!(planned.blend, vec![(1, 90), (3, 70)]);
    }

    #[test]
    fn a_stopped_upstream_submits_the_native_table_only() {
        let mut native = source("native", 50, &[(1, 5_000), (6, 4_000), (12, 4_500)]);
        native.lag_exempt = true;
        let down = [source("mempool-space", 50, &[])];
        let planned = plan(&FeesConfig::default(), native, &down);
        assert!(planned.without_upstream);
        assert_eq!(planned.blend, vec![(1, 5_000), (6, 4_000)]);
        assert_eq!(planned.blend_min, vec![(1, 5_000), (6, 4_000), (12, 4_000)]);
        assert!(targets_to_ask(&[]).contains(&1));
        assert!(targets_to_ask(&[2, 9]).contains(&9));
    }

    #[test]
    fn config_keeps_the_priority_list_and_scalars_can_override() {
        let dir = std::env::temp_dir().join(format!("blvm-fees-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            r#"
priority = ["mempool-space", "native", "blockstream"]
max_height_delta = 3
request_timeout_ms = 5000
submit_interval_secs = 10
"#,
        )
        .unwrap();
        let mut cfg = FeesConfig::load(&path).unwrap();
        assert_eq!(cfg.priority[0], "mempool-space");
        cfg.apply_scalar_overrides(None, None, Some("4")).unwrap();
        assert_eq!(cfg.submit_interval_secs, 4);
        assert!(cfg.apply_scalar_overrides(None, None, Some("0")).is_err());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
