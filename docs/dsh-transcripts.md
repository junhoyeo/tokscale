# DSH transcript coverage

Tokscale streams DSH's plain JSONL and concatenated Zstandard transcripts. The physical encoding is detected from the Zstandard magic, so the filename does not need to match the encoding. Transcripts larger than the former 64 MiB whole-file ceiling are included without retaining the full decoded transcript in memory.

The reader limits each file to 1 GiB on disk and 1 GiB decoded, each record to 64 MiB (including its line ending), and the Zstandard history window to 64 MiB. These bounds limit work and allocations from damaged or unusually large inputs. Parsed usage messages and their accounting identities are still retained for settlement replacement and cross-file deduplication; memory therefore also scales with the number of usage records, not just the input buffer.

An unreadable source, invalid JSON, interrupted compressed frame, or reached limit produces a per-scan warning. Complete readable records before a read/decode limit or error remain counted; a malformed JSON record is skipped and reading continues. An oversized record stops the source because it could contain routing, retry, or fork state needed to interpret later usage safely. A valid final JSON record need not end with a newline.

Incomplete scans are never saved as complete source-cache entries, and a partial rewrite invalidates a previous entry. Each subsequent scan retries the source. Repairing or completing the transcript clears its warning and makes it cacheable again. The DSH parser generation invalidates older cached prefixes once after upgrading.

Request identities, fork ownership, retry accounting, compaction calls, billing provenance, and generation timing use the same rules for plain and compressed input. The change does not fetch remote history, modify DSH files, or reconstruct usage that DSH did not record.
