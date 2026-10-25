# Shared storage design

Status: Engineering proposal supporting accepted ADRs 0059 and 0060; no implementation or durability qualification performed. Volume-backed storage and a native API are accepted; detailed mechanics below remain proposed.

## Shape and ownership

Use one storage module inside the orchestrator, backed by PostgreSQL metadata and an exclusively owned PVC filesystem. Consumers use opaque identities and authenticated operations. They never know storage paths. Agent workspaces remain separate; publishing creates a snapshot rather than a live link to a changing workspace file.

The skills, conversations, and tasks modules own why a file exists and its authorized audience. The storage module owns bytes, integrity, transfers, and reclamation. This distinction prevents a shared physical store from becoming a globally readable bucket.

Proposed domain concepts:

| Concept | Meaning |
| --- | --- |
| Upload | An authorized, bounded transfer attempt with an identity, destination scope, expiry, and idempotency key. |
| Blob | Immutable bytes identified by an opaque ID, with server-computed digest and length. |
| Artifact reference | A domain-owned link to a blob, including display metadata and the applicable resource scope. |
| Installation quota | Admission and storage limits covering staging bytes as well as retained content. |

Initially use independently identified blobs rather than global content deduplication. Digests verify content, not authorization. Human filenames are metadata and never become caller-controlled storage paths. Replacing a deliverable or revising a skill creates new bytes and a new revision; it does not overwrite a published blob.

## Caller experience

Conceptual operations, not selected HTTP paths:

```text
upload = begin_upload(destination, declared_size, idempotency_key)
send_bytes(upload.id, byte_stream)
blob = finalize_upload(upload.id)
attach_to_resource(destination, blob.id, expected_resource_revision)
download(resource_reference, optional_byte_range)
```

A convenience upload operation can hide these steps for small files while preserving the same identities and recovery contract. Status lookup resolves an uncertain finalize response. An idempotency key is scoped to the actor and request intent; reusing it with different content or destination is rejected. Arbitrary multipart upload and resumable chunk protocols are deferred until size targets justify them. A failed basic stream can restart under the same logical operation without pretending partial bytes are complete.

Begin-upload checks destination authority and reserves capacity. Finalize and attachment recheck current authority. Only a ready blob owned by an eligible upload may be attached. Attaching to a different resource requires explicit destination authority. Physical storage identity is never sufficient to download a file; read authorization comes from a valid domain reference or narrowly scoped upload ownership.

## Publication and recovery

Proposed upload states are receiving, publishing, ready, failed, and expired. Deletion has a separate lifecycle. Transitions are durable and serialized per upload so finalize, expiry, retry, and recovery cannot compete for the same files.

1. Persist the upload identity and capacity reservation before receiving content.
2. Stream into a uniquely created staging file on the same filesystem as final storage. Enforce the byte ceiling while reading and compute a digest incrementally. Do not buffer the whole object in memory or hold a database transaction open for the transfer.
3. Check completion and any supplied expected length or checksum. Record enough publication intent to identify the expected final blob during recovery.
4. Flush application buffers, synchronize file data and required metadata, publish into a unique final location without overwriting an existing object, and synchronize affected directories. Qualify the exact sequence on the supported filesystem and storage driver.
5. Commit ready metadata in PostgreSQL after durable file publication. Only then acknowledge completion. Attach domain references in a database transaction that verifies the blob remains ready and is not being deleted.

Crash behavior must favor recoverable unreferenced bytes over metadata falsely promising a complete file. A crash before publication leaves a staged upload; after publication but before the database commit, it leaves a recoverable final file tied to upload intent. Recovery verifies identity, size, and digest before completing a pending transition. A response lost after commit returns the same blob on status lookup or an idempotent retry.

Missing or corrupt ready content is an integrity fault. Report it and identify affected references; never manufacture an empty successful response or silently mark data restored. Disk errors and synchronization failures must propagate before success is acknowledged.

Rust's ordinary `rename` can replace an existing destination and cannot cross mount points. The implementation needs a qualified publication method that cannot overwrite, with staging on the same filesystem. Closing a file is insufficient evidence of durability; explicit synchronization errors must be handled. Linux also requires attention to directory entries, beyond synchronizing the file. [Rust rename](https://doc.rust-lang.org/std/fs/fn.rename.html), [Rust File synchronization](https://doc.rust-lang.org/std/fs/struct.File.html#method.sync_all), [Linux fsync](https://man7.org/linux/man-pages/man2/fsync.2.html).

## Serving and resource limits

Stream uploads and downloads with bounded buffers and backpressure. Run blocking filesystem work through a bounded execution path, with separate admission limits from latency-sensitive control operations. Apply per-actor and installation limits so one large transfer cannot exhaust descriptors, memory, or storage. Limit archive expansion separately from compressed upload size.

Track staged bytes, retained bytes, free capacity, active transfers, failures, and transfer latency. Reserve operational headroom for journals and recovery. Downloads recheck access at admission; policy for revocation during an existing stream remains open. Browser responses need deliberate content-type, download, and origin handling so uploaded HTML or scripts cannot execute with console privileges.

Object reads require orchestrator availability. Agents may keep authorized downloaded copies in their own workspaces, but these are not another authoritative object store. Existing offline-authority limits still apply.

## Cleanup, backups, and deployment

[ADR 0064](../adr/0064-coordinated-backup-and-restore.md) selects Cubebot-owned backup and restore coordination. [ADR 0065](../adr/0065-maintenance-window-backups.md) selects a coordinated maintenance window. The specific capture and synchronization mechanics below remain proposed.

Removing a domain reference does not immediately unlink shared bytes. A transaction marks an eligible unreferenced blob for deletion and blocks new references; a retryable worker removes bytes and records completion. Coordinate deletion with active transfers and backups. Do not reclaim a file merely because one directory scan failed to match it to metadata.

The backup procedure pauses admission and relevant writers, settles in-flight publication, establishes a matching database and volume snapshot or verified copy, records a manifest, and releases the pause only when the backup boundary is secured. Backend-specific snapshot support and agent workspace checkpoint mechanics remain open. The pause can extend through file copying where no suitable snapshot is available. Restore must verify both records and bytes before claiming recovery. Retained database restore points constrain when old blobs can be physically deleted.

Prefer a CSI-backed volume with single-pod access where supported, while retaining the installation's explicit single-active-owner contract. Kubernetes `ReadWriteOnce` permits multiple pods on the same node; `ReadWriteOncePod` provides a stronger pod restriction with supported drivers. Neither defines the complete application recovery procedure. [Kubernetes persistent-volume access modes](https://kubernetes.io/docs/concepts/storage/persistent-volumes/#access-modes).

The storage backend determines node-loss durability, reattachment behavior, expansion, and snapshot capabilities. Deployment documentation must state required semantics and retention behavior on uninstall. Supporting one PVC does not establish a throughput target: test concurrent small artifacts, large uploads, full disk, process crashes, node replacement, and restore while measuring control-plane latency.

## Selected protocol direction

[ADR 0060](../adr/0060-native-storage-api.md) selects a native Cubebot interface using platform identities and domain permissions, exposed to agents through native tools. S3 wire compatibility is outside the initial design. Exact transport and schemas remain open. The selected interface does not imply distributed replication or an arbitrary shared POSIX filesystem.
