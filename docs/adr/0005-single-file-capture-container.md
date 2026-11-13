# ADR 0005: `.lacap` is a single-file container, not a ZIP

Status: accepted

## Context
Captures reach several GB, and opening a 1e9-sample capture in under 2 s needs random access to
single chunks. A ZIP needs ZIP64 for that size and a central directory to find entries; most ZIP
libraries also work on whole entries rather than offsets.

## Decision
`.lacap` is our own single-file container (doc 09):

- a fixed header (magic, format version);
- chunk, LOD and annotation blobs, each with a small header and a CRC-32;
- a compressed JSON index at the end, followed by a fixed footer that points to it.

Readers read the footer and index, then blobs by offset on demand. Writers append blobs and write
the index last; saving after edits appends a new index ("save as" writes a compact copy). A new
capture is recorded straight into a working `.lacap`, so saving costs almost nothing.

## Consequences
- Random access and files over 4 GB with plain file handles.
- Opening and saving are close to free: the engine reads chunks straight from the file.
- The file cannot be inspected with `unzip`; `logica inspect <file>` prints the index instead.
- The format is ours to document and version; doc 09 is its specification.
