# ADR 0004: Chunked transition storage with per-chunk level of detail

Status: accepted

## Context
Signal Detective stores each channel as one array of global 32-bit edge indices: simple and fast,
but capped at 2^32 samples, not cheaply appendable, and slow to draw zoomed out on very dense
channels. Captures here reach several billion samples per channel and grow while capturing.

## Decision
Store transitions per channel in fixed chunks of 2^24 samples, as chunk-relative 32-bit offsets,
falling back to raw bits for chunks denser than the raw form, and nothing for quiet chunks. Keep a
coarse level of detail resident for every active chunk and a fine one for dense chunks only, so LOD
memory follows activity. Details in doc 04.

## Consequences
- No length limit below 2^53 samples; memory proportional to activity, never above the raw size.
- Appending while capturing is cheap; eviction to disk is per chunk.
- Every reader (measurements, detectors, decoders, the waveform) goes through chunk-aware cursors
  and edge streams.
