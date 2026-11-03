# Draft-and-Verify: Speculative Decoding for Autoregressive Language Models

**Authors:** L. Ferreira, M. Okonkwo, S. Halvorsen — synthetic abstract for demonstration.

## Abstract

Autoregressive decoding generates one token per forward pass, leaving accelerator memory bandwidth as the binding constraint at small batch sizes. We propose Draft-and-Verify, in which a small draft model proposes a block of k candidate tokens that the target model then scores in a single parallel forward pass; accepted tokens are emitted and the first rejected position is resampled from the target distribution, preserving the target model's output distribution exactly. On a 70B-parameter target with a 1B draft model, Draft-and-Verify achieves a 2.4x wall-clock speedup on summarization and 2.1x on code generation at batch size 1 on a single 80GB accelerator, with no change in greedy or sampled output quality as measured by exact-match agreement against unassisted decoding. The speedup degrades to 1.3x at batch size 32, where compute rather than bandwidth dominates, and depends on draft acceptance rate, which fell from 0.78 on English prose to 0.61 on mathematical text. We release drafting heuristics for choosing k adaptively.

## Evaluation

Models: Llama-class 70B target, 1B draft trained on the same corpus. Hardware: one 80GB HBM accelerator. Benchmarks: CNN/DailyMail summarization, HumanEval-style code completion, GSM8K-style math. Metric: tokens per second and exact-match agreement with baseline decoding.
