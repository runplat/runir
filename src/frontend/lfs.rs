use serde::{Deserialize, Serialize};

use crate::wire::Annotate;

#[derive(Debug, Serialize, Deserialize)]
pub struct LFS {

}

impl Annotate for LFS {
    fn type_name() -> &'static str {
        "runir::frontend::lfs"
    }
}