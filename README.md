# RUNIR - Runtime Intermediate Representation

`runir` is a data framework for building runtime-friendly representations of data. Its core priorities are developer ergonomics, correctness, and performance.

The framework adopts a **plumbing and porcelain** architectural model, inspired by Git. *Plumbing* refers to the foundational components that handle raw, system-level concerns, while *porcelain* provides the higher-level, developer-facing interfaces built on top.

To validate and demonstrate the plumbing layer, `runir` includes porcelain frontends such as the `kv` module, a key-value store interface built on the `runir` data model.

By building on top of the plumbing, the `kv` frontend delivers a content-addressable data store with querying, indexing, and archiving built in.

To interact with the data-model and frontends, this crate also provides a cross-platform CLI tool `runir` that can be used in scripting or personal workflows.

## Roadmap

This library is currently in an early **alpha-1** state. Breaking changes may occur in alpha and beta releases until a stable version is reached.. Below are the major goals of each release milestone.

- **alpha-2**
    - Introduce `wire` protocol to plumbing
- **beta-1**
    - Introduce a `catalog` and `vault` frontend modules
- **beta-2**
    - Complete `runir` cli tool
- **stable**
    - Finalize data format and documentation

## Getting Started

To use the cli tool,

```sh
cargo install runir --features cli --version 0.1.0-alpha1
```

To use the library,

```sh
cargo add runir --version 0.1.0-alpha1
```

### CLI Examples

**Using the `kv` frontend**

```sh
# Put a JSON object into the store under the record label "example" under the default namespace
runir kv put --json example <<EOF
{ "value": "hello world" }
EOF

# Print the "value" property from the record "example" under the default namespace
runir kv get --peek value example

# Prints metadata on the stored record "example" in TOML format
runir kv info example
```

### Plumbing Examples

> **Note**
>
> When working with records, the majority of functions are provided by extension traits. It is recommended to use `use runir::prelude::*;` to import these traits.
>

**Creating a Record that stores a TOML object**

This is a simple demonstration of creating a record w/ `runir` that stores an "object".

> **Note**
>
> Internally, `runir` normalizes all objects as a `flexbuffer` root which enables zero-copy reads of values.

```rust
let ns = Namespace::default();

// Create an "example" record and store TOML using the toml! macro
let rec = ns.store("example", &toml!{ value = "hello world" });

// Since an object is stored in the Record, fields can be accessed without deserializing the object
assert_eq!("hello world", rec.field("value").str().unwrap());

// This uses the `Peek` extension interface to convert the record into a toml::Value
use runir::prelude::*;

let val = rec.to_obj::<toml::Value>();
```

### Frontend Examples

**Using the `kv` frontend to store values**

See the `kv` module documentation for more details.

```rust
let mut kv = runir::kv::new();

// This shows putting a binary value, but you can also put objects or structured data.
kv.put("value", b"hello world")?;

// The value is always available immediately
assert_eq!(b"hello world", kv.get("value").unwrap());

// Once save(..) is called, the same data will be persisted to disk and available on process restart
kv.save().await?;
```

## Architecture 

See [architecture.md](/docs/architecture.md) for architecture details

## License

MIT