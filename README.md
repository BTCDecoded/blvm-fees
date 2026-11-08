# blvm-fees

Publishes the `blend` and `blend-min` fee estimators. `blend` follows LN-Zap `mergeFeeEstimates`. `blend-min` takes the per-target minimum and then a prefix minimum.

The module reads `blvm.upstream.v1` when that module is loaded. If upstream data is missing, it still submits a table built from the node's native quotes.

It is not a default module pin. External fee data stays off until the operator selects an estimator.
