pub struct Opts {
    record: RecordOpts
}

bitflags::bitflags! {
    /// Record options
    #[derive(Copy, Clone, Debug)]
    pub struct RecordOpts : u8 {
        /// Indicates that the record can be processed by the indexer
        const Indexing = 1;
        /// Indicates that the record should not be archived
        const NoArchive = 1 << 1;
    }
}
