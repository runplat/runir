/// Extension trait for types to remove padding bytes
pub trait TrimPadding {
    /// Padding symbol to remove
    const PADDING_SYMBOL: u8 = b'\0';

    /// Returns a slice without trailing padding bytes
    fn trim_padding(self) -> Self;
}

impl TrimPadding for &[u8] {
    fn trim_padding(self) -> Self {
        let padding = self
            .iter()
            .rev()
            .take_while(|b| **b == Self::PADDING_SYMBOL)
            .count();
        &self[..self.len() - padding]
    }
}

mod tests {
    #[test]
    fn test_trim_padding() {
        use super::TrimPadding;
        let arr: &[u8] = &[2, 0, 1, 0, 5, 0, 0, 7, 0, 0, 0, 0, 0, 0];
        assert_eq!(arr.trim_padding().len(), 8);
    }
}