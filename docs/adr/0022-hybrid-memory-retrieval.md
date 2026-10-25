# ADR 0022: Hybrid memory retrieval

Status: Accepted

Date: 2026-10-23

Long-lived agents need to recall precise terms and related concepts expressed in different words. Derived memories and retained source history cannot all fit in each model call.

Decision: Use hybrid retrieval, combining keyword search, semantic similarity, and structured filters. The user selected this model on 2026-10-23. The memory module combines eligible candidates into a bounded result set with provenance, rather than requiring the caller to coordinate separate search systems.

Usage: A query about data residency can retrieve a relevant memory phrased as keeping customer data in Brazil. A query for a specific project identifier can use lexical matching or identifier filters. Results include enough identity and source information to inspect or retrieve the supporting evidence. These are target scenarios for evaluation, not guarantees about a chosen embedding model.

The caller supplies its authenticated agent context, query, optional filters, and result budget. The orchestrator derives or validates memory ownership from the authenticated request. Candidate retrieval and ranking must respect the permitted ownership scope. Hybrid search does not create a shared installation-wide memory pool or allow caller-supplied identifiers to bypass access checks.

Keyword matching supports lexical recall, semantic representations support conceptual similarity, and structured metadata narrows eligible records. Exact matching, ranking fusion, deduplication, recency handling, and treatment of superseded memories will be specified and evaluated separately. Similarity is a retrieval signal, not evidence that a memory is correct.

Implementation direction: Use PostgreSQL's built-in full-text search and pgvector for vector similarity in the existing structured store. This avoids a separate search database. Extension version, embedding provider or local model, vector representation, and index configuration remain open implementation choices. Hybrid retrieval is the accepted contract; detailed dependency qualification is still required. No additional LLM reranker is required initially.

Ownership: The memory module owns retrieval, candidate combination, filtering, and the result contract. Context assembly owns selection within the model's overall context budget. Search indexes and embeddings are derived data; source records and memory provenance remain authoritative under [ADR 0020](0020-memory-consolidation-and-source-history.md).

Alternatives considered: Keyword search and structured filters alone are simpler but may miss paraphrased concepts. Semantic search alone can miss precise references and return loosely related material. The user selected their combination.

Consequences: Embedding generation adds computation and model lifecycle management. Records can exist before their embeddings are ready. Degraded retrieval, indexing lag, model changes, deletion propagation, and reindexing require explicit behavior. Retrieval quality must be measured using both precise-reference and conceptual-recall cases.

Related decision: [ADR 0023](0023-tool-initiated-memory-retrieval.md) selects explicit agent tool calls as the trigger for retrieval, with no automatic saved-memory injection before turns.

Open details: Embedding selection and placement; indexing cadence; memory versus source-history search coverage; ranking; fallback behavior; language support; result limits; and recall, latency, and cost targets.

Validation scenarios: Exact project reference; paraphrased concept; no useful match; duplicate candidates from both search paths; superseded information; new memory awaiting embedding; deleted memory excluded from results; and cross-agent access denied before results are exposed.

Implementation references reviewed during design: [PostgreSQL full-text search](https://www.postgresql.org/docs/current/textsearch-intro.html) and [pgvector](https://github.com/pgvector/pgvector).
