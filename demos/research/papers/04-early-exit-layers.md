# Confident Tokens Leave Early: Adaptive Depth for Decoder-Only Transformers

**Authors:** P. Varga, N. Okoro — synthetic abstract for demonstration.

## Abstract

Not every token needs every layer. We train lightweight exit heads after every fourth layer of a decoder-only transformer and let a token stop propagating once an exit head's confidence exceeds a calibrated threshold, copying its hidden state forward so that later tokens can still attend to it. On a 7B model, 41% of tokens exit before the midpoint, yielding a 1.6x reduction in average FLOPs per token and a 1.35x wall-clock speedup at batch size 8, with a 0.7-point drop in average accuracy across six reasoning benchmarks. The technique requires fine-tuning the exit heads (about 2% of the pre-training compute) and interacts poorly with speculative decoding, whose verification pass assumes a fixed depth. Speedups are also smaller on hardware with high fixed per-layer launch overhead, where skipping a layer saves less than its proportional share of time.

## Evaluation

Models: 1.3B and 7B decoder-only transformers with exit heads every 4 layers. Benchmarks: six reasoning and QA suites. Hardware: one 80GB accelerator. Metrics: FLOPs per token, tokens per second, accuracy delta versus the full-depth model.
