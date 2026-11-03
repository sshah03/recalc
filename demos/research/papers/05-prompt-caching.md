# Sharing Prefixes Across Requests: Prompt Caching in Multi-Tenant Serving

**Authors:** E. Duarte, K. Sato, M. Bergström — synthetic abstract for demonstration.

## Abstract

Production traffic is highly repetitive at the prefix: system prompts, few-shot examples, and retrieved documents recur across thousands of requests. We describe a serving-layer cache that stores the attention state of common prefixes in a radix tree keyed by token sequence and attaches new requests to the longest matching path, so that only the unshared suffix is prefilled. On a multi-tenant trace with a median shared prefix of 1,800 tokens, the cache eliminates 71% of prefill compute and cuts median time-to-first-token from 840 ms to 190 ms on a 34B model, while raising sustained throughput by 2.3x. Hit rate depends on tenant mix: a workload with per-user personalization early in the prompt achieves only a 22% hit rate, and cache memory competes directly with the KV cache of in-flight requests, so we introduce a cost-aware eviction policy that trades hit rate for batch size. Prefix reuse also requires exact token-level matches, so small template changes invalidate the cache for a tenant.

## Evaluation

Models: 8B and 34B. Workload: an anonymized multi-tenant trace of 1.2M requests. Hardware: eight 80GB accelerators. Metrics: prefill FLOPs saved, time-to-first-token, throughput, cache hit rate by tenant.
