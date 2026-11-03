# Two-Bit Keys, Four-Bit Values: Asymmetric KV-Cache Quantization for Long Contexts

**Authors:** R. Adeyemi, T. Lindqvist — synthetic abstract for demonstration.

## Abstract

The key-value cache grows linearly with context length and, at 128K tokens, exceeds the size of the model weights for mid-sized transformers. We study post-training quantization of the cache and find that keys and values tolerate different precisions: keys exhibit outlier channels that make uniform low-bit quantization lossy, while values are near-Gaussian. Our scheme quantizes values to 4 bits per element and keys to 2 bits after a per-channel rotation that spreads outliers, reducing cache memory by 5.3x relative to 16-bit storage. On a 13B model at 64K context this raises the maximum serving batch size from 6 to 31 on a 48GB device and improves throughput by 3.9x, at a cost of 0.4 points of perplexity on a long-document held-out set and a 1.1-point drop on a needle-in-a-haystack retrieval suite. The method adds a rotation matmul per layer that costs 3% of decode latency, and it does not compose with attention kernels that assume 16-bit keys without modification.

## Evaluation

Models: 7B and 13B decoder-only transformers. Contexts: 8K to 128K tokens. Hardware: 48GB accelerator. Metrics: perplexity on long-document corpus, needle-in-a-haystack accuracy, throughput in tokens per second, peak memory.
