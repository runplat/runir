mod worker;
mod db;
mod block;
pub mod archive;

#[cfg(test)]
mod test {
    use bytes::{BufMut, BytesMut};
    
    #[test]
    fn buffer() {
        let mut builder = flexbuffers::Builder::default();
    
        let mut vec = builder.start_vector();
        vec.push(10);
        vec.end_vector();
    
        let mut bytes = BytesMut::new();
        let vec = builder.take_buffer();
        bytes.put(&vec[..]);
    }    
}