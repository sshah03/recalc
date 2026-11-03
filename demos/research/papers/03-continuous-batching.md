# Iteration-Level Scheduling for LLM Serving

**Authors:** H. Nakamura, B. Castellanos, A. Mwangi — synthetic abstract for demonstration.

## Abstract

Request-level batching pads every sequence in a batch to the longest, so a single long generation holds accelerator capacity hostage for shorter neighbors. We present an iteration-level scheduler that admits and retires requests at every decoding step, paired with a paged attention layout that lets sequences of different lengths share physical memory blocks without fragmentation. On a production-like trace with a heavy-tailed output-length distribution, the scheduler raises goodput (requests per second meeting a 200 ms per-token latency target) by 8.2x over request-level batching on a 13B model and by 3.6x over a static-batching baseline with a tuned batch size. Median time-to-first-token falls from 1.9 s to 0.31 s under load. The gains shrink to 1.4x when output lengths are uniform, and preemption of long sequences under memory pressure introduces tail-latency spikes of up to 2.7 s for the preempted requests, which we mitigate but do not eliminate with a recompute-versus-swap policy.

## Evaluation

Models: 7B, 13B, 34B. Workload: a synthetic trace replaying the length distribution of a public chat dataset. Hardware: four 80GB accelerators. Metrics: goodput at a latency SLO, median and p99 time-to-first-token, p99 inter-token latency.
