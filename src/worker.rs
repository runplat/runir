use flexbuffers::Builder;
use sha2::Sha256;

/// Struct containing db worker state
#[derive(Default)]
pub struct Worker {
    /// Builder for mapping data into flexbuffer entries to store
    builder: Builder,
    /// Digester which is used to create an address to the content produced by the builder
    digest: Sha256
}

#[cfg(test)]
mod test {
    #[test]
    fn test_worker() {
    }
}
