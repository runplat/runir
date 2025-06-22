use scalable_cuckoo_filter::ScalableCuckooFilter;
use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

/// Text metadata for a text-based field
#[derive(Serialize, Deserialize)]
pub struct TextMetadata {
    /// Filter produced by tokenizing the text
    filter: ScalableCuckooFilter<str>,
    /// Length of the indexed text in bytes
    len: u64
}

impl TextMetadata {
    /// Given a query string, returns true if the text metadata contains and words from
    /// the query
    #[inline]
    pub fn contains(&self, query: &str) -> bool {
        for word in query.unicode_words() {
            if self.filter.contains(word) {
                return true;
            }
        }
        false
    }

    /// Returns the length of text metadata
    #[inline]
    pub fn text_len(&self) -> u64 {
        self.len
    }
}

impl From<&str> for TextMetadata {
    fn from(value: &str) -> Self {
        let mut filter = ScalableCuckooFilter::<str>::new(100, 0.001);
        for word in value.unicode_words() {
            filter.insert(word);
        }
        filter.shrink_to_fit();
        TextMetadata { filter, len: value.len() as u64 }
    }
}

#[cfg(test)]
mod test {
    use super::TextMetadata;

    #[test]
    fn test_text_metadata() {
        let text_meta = TextMetadata::from("The quick brown fox");
        assert!(!text_meta.contains("elementary"));
        assert!(text_meta.contains("brown"));
        assert!(!text_meta.contains("bro"));
    }
}