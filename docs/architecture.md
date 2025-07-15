# Architecture Overview

Modern data systems are increasingly composed of loosely coupled pipelines: ephemeral services, scheduled jobs, intermediate formats, and domain-specific indices stitched together with ad-hoc logic. Despite all this flexibility, one thing remains frustratingly hard to do well: **track, mutate, and version data in a way that is reliable, inspectable, and composable across boundaries**.

Most systems either bake in too many assumptions — like schemas, query languages, or storage backends — or push the burden of lifecycle management, deduplication, and auditability onto the user. This project was born out of a desire to **standardize the storage and mutation of structured data at the level of records**, without committing to any one view of how that data is interpreted or retrieved.

---

## Record System

runir approaches this problem by treating **records** as the atomic unit of truth: immutable, content-addressed, and explicitly annotated with lifecycle and behavioral metadata.

Instead of modeling application data directly, runir models how data moves — how it gets written, mutated, archived, and indexed. Each record carries enough embedded context to describe how it should be handled by downstream systems without requiring coordination or global state.

The goal is not to replace databases, file formats, or message queues — but to provide a **common substrate** that can safely power all of them, one durable, self-describing record at a time.

---

## Porcelain vs. Plumbing

The design of runir is heavily influenced by the distinction between **plumbing** and **porcelain**, a concept borrowed from Git. Plumbing refers to the low-level, reliable, composable building blocks — commands and primitives that do exactly what you tell them, without trying to abstract or hide complexity. Porcelain, by contrast, refers to the user-facing tooling built on top of those internals: friendly commands, interfaces, or integrations that present a higher-level mental model.

The bulk of runir is **plumbing**. It doesn't dictate how your data is queried, visualized, or transformed — it just ensures that every piece of data has a clear identity, an explicit lifecycle, and a safe way to mutate or transport it.

Because of this design, runir provides:

- Idempotent mutation modeled through append-only records  
- Content-addressed storage using cryptographic digests  
- Branch-aware lifecycle semantics, including staging and soft deletion  
- Streamable, zero-copy archive formats suitable for packing, scanning, and transport  
- Configurable runtime behavior to support indexing, archival, and ingestion flows

Higher-level systems — indexing layers, object stores, event pipelines — can be built on top of this model without redefining core concepts like mutation, deduplication, or archival. runir provides a consistent substrate that remains agnostic to how records are interpreted or queried.

---

## Data Model

runir also supports a normalized, self-describing object format that allows users to bring in their own types with minimal overhead. Instead of requiring reflection, dynamic dispatch, or allocation-heavy deserialization to access fields, records can be queried directly — including by field path — without fully decoding the underlying object.

This makes it possible to build performant systems on top of runir using your own data types, while still benefiting from compact storage and introspectable structure. Whether you're storing entities, events, or domain-specific documents, you retain full control over serialization without needing to define schemas up front or rely on runtime code generation.
