mod extent;
pub use extent::RecordExtent;

mod data;
pub use data::VirtualData;
pub use data::VirtualDataSlim;

pub mod vol;

mod object;
pub use object::ObjectEncoder;