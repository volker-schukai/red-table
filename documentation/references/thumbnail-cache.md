# Thumbnail cache

## Two cache layers

`red-table` uses two complementary cache layers:

1. Protocol-neutral PNG thumbnails persist according to version 0.9 of the
   [freedesktop.org Thumbnail Managing Standard](https://specifications.freedesktop.org/thumbnail/latest-single/).
2. Terminal protocol, cell-size, and quality variants remain in a memory cache
   bounded to both 256 entries and 128 MiB of conservatively accounted prepared
   data. Raw thumbnail resize targets are rejected above 64 MiB.

The disk layer is located below `$XDG_CACHE_HOME/thumbnails`. When
`XDG_CACHE_HOME` is unset or blank, the location is `$HOME/.cache/thumbnails`.
It uses the standard `normal`, `large`, `x-large`, and `xx-large` directories for
maximum dimensions of 128, 256, 512, and 1024 pixels. Targets requiring more than
1024 pixels bypass persistence to avoid silently reducing requested quality.

The status field `D<hits>/<misses>` reports persistent hits and misses completed
during the current process. It does not include in-memory protocol-cache hits.
Debug status reports the memory layer as `M<entries>/<MiB>`.

Run `task benchmark-thumbnail-cache` inside the development shell to compare
cold original decoding and persistence with validated warm reuse. The benchmark
uses a deterministic `1920x1080` PNG, a `320x180` target, and seven samples per
path.

## Identity and validity

The persistent filename is the lowercase MD5 digest of the canonical absolute
file URI followed by `.png`, as required by the standard. MD5 identifies the URI;
it is not used as a security or source-content hash.

Every red-table entry contains:

- `Thumb::URI`
- `Thumb::MTime`
- `Thumb::Size`
- `X-RedTable::MTimeNsec`
- `X-RedTable::Schema`

A hit requires exact URI, whole-second modification time, and byte size. Entries
created by red-table additionally require the exact nanosecond modification time
and processing schema. Standards-compliant entries from other applications are
accepted without the red-table fields, but entries without `Thumb::Size` are
conservatively regenerated.

Source metadata is read before and after decode/resize. If it changes, the
generated pixels are discarded and retried once. Cache files are written to a
unique temporary sibling, flushed, and atomically renamed; no `fsync` is issued
because the cache is regenerable and a truncated entry is detected and rewritten.
Sources larger than twice the size class are box-reduced before the Lanczos3
pass. On Unix, cache directories use mode `0700` and files use `0600`.

Corrupt files, permission failures, full filesystems, or unavailable cache homes
degrade to original-image decoding. They do not stop browsing.

## Limits and removal

The memory layer accounts encoded Kitty, Sixel, and iTerm2 string capacities and
the complete Halfblocks cell array. It evicts least-recently used variants
until both limits are satisfied. A single object above the byte limit becomes a
retained error rather than entering an allocation/retry loop. A previous quality
is displayed only while its exact replacement remains pending.

Metadata validation cannot detect a source replacement that deliberately
preserves canonical path, byte size, and nanosecond modification time. Hashing
every full source would defeat warm-cache latency for large collections, so that
case is intentionally outside the current contract.

The freedesktop thumbnail directory is shared by desktop applications. Removing
one or more of its standard size directories is safe because they contain only
regenerable cache data, but it also removes thumbnails belonging to other
applications. Stop thumbnail-producing applications before manual cleanup to
avoid racing their atomic writers. Automatic size- or age-based pruning is not
implemented by red-table yet.
